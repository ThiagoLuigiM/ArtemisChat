import { useState, useEffect, useRef, useCallback, useMemo } from "react";
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

interface PhraseTemplate {
  situation: string;
  template: string;
  occurrences: number;
}

interface CartilhaTokenEvent {
  content: string;
}

interface CartilhaImage {
  /** Bytes da imagem (Uint8Array → Tauri serializa como Vec<u8> sem precisar base64) */
  bytes: number[];
  /** Extensão sem ponto, ex: "png", "jpg" */
  extension: string;
  /** Legenda escrita pelo dev */
  caption: string;
  /** Apenas para preview no React (data URL) — NÃO enviado ao backend */
  previewUrl: string;
}

interface TestScenariosSuggestion {
  happy_path: string;
  edge_cases: string;
  negative_cases: string;
  acceptance_criteria: string;
  regression_areas: string;
  risks: string;
}

/** Palavras-chave que indicam mudança de fluxo/UI — quando o campo "O que foi alterado"
 *  contém qualquer uma, a cartilha exige imagens. */
const FLOW_KEYWORDS = [
  "novo fluxo",
  "nova tela",
  "novo botão",
  "novo botao",
  "nova aba",
  "nova permissão",
  "nova permissao",
  "novo menu",
  "novo cadastro",
  "nova rota",
  "nova navegação",
  "nova navegacao",
  "mudança de fluxo",
  "mudanca de fluxo",
  "mudança de menu",
  "mudanca de menu",
  "novo parâmetro",
  "novo parametro",
];

function cartilhaImagesRequired(form: FormFields): { required: boolean; reason: string | null } {
  if (form.parametro.trim()) {
    return { required: true, reason: "campo 'Novo parâmetro ou permissão' preenchido" };
  }
  const text = form.correcao.toLowerCase();
  const matched = FLOW_KEYWORDS.find((kw) => text.includes(kw));
  if (matched) {
    return { required: true, reason: `o campo 'O que foi alterado' menciona '${matched}'` };
  }
  return { required: false, reason: null };
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
  onGenerateCartilha,
  onGenerateTestes,
  disabled,
}: {
  fields: FormFields;
  onChange: (f: FormFields) => void;
  onGenerate: () => void;
  onGenerateCartilha: () => void;
  onGenerateTestes: () => void;
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

      <div className="form-footer form-footer-multi">
        <button
          type="button"
          className="btn-secondary"
          onClick={() => onChange(EMPTY_FORM)}
          disabled={disabled}
        >
          Limpar
        </button>
        <div className="form-footer-right">
          <button
            type="button"
            className="btn-secondary"
            onClick={onGenerateCartilha}
            disabled={disabled || !canGenerate}
            title="Gera uma cartilha HTML didática a partir desse input (com imagens, se aplicável)"
          >
            Cartilha
          </button>
          <button
            type="button"
            className="btn-secondary"
            onClick={onGenerateTestes}
            disabled={disabled || !canGenerate}
            title="Monta um formulário estruturado para a equipe de testes/QA"
          >
            Form de testes
          </button>
          <button
            type="button"
            className="btn-primary"
            onClick={onGenerate}
            disabled={disabled || !canGenerate}
          >
            {disabled ? "Gerando..." : "Devolutiva (N1)"}
          </button>
        </div>
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

type LearningTab = "evitar" | "estilo" | "campos" | "frases";

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
        <button
          type="button"
          className={`tab ${tab === "frases" ? "active" : ""}`}
          onClick={() => setTab("frases")}
        >
          frases-modelo
        </button>
      </div>

      {tab === "evitar" && <EvitarTab vaultPath={vaultPath} />}
      {tab === "estilo" && <EstiloTab vaultPath={vaultPath} />}
      {tab === "campos" && <CamposTab vaultPath={vaultPath} />}
      {tab === "frases" && <FrasesTab vaultPath={vaultPath} />}
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

function FrasesTab({ vaultPath }: { vaultPath: string | null }) {
  const [analyzing, setAnalyzing] = useState(false);
  const [templates, setTemplates] = useState<PhraseTemplate[] | null>(null);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [applying, setApplying] = useState(false);
  const [appliedFile, setAppliedFile] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const handleAnalyze = async () => {
    setError(null);
    setTemplates(null);
    setAppliedFile(null);
    setAnalyzing(true);
    try {
      const out = await invoke<PhraseTemplate[]>("analyze_phrase_templates");
      setTemplates(out);
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
    if (!templates || selected.size === 0) return;
    setApplying(true);
    setError(null);
    try {
      const chosen = templates.filter((_, i) => selected.has(i));
      const file = await invoke<string>("apply_phrase_templates", { templates: chosen });
      setAppliedFile(file);
      setTemplates(null);
      setSelected(new Set());
    } catch (e) {
      setError(String(e));
    } finally {
      setApplying(false);
    }
  };

  return (
    <>
      <p className="help">
        O Artemis envia até 80 devolutivas aprovadas para a IA, que identifica frases
        que se repetem (com variações de datas/versões/caminhos) e propõe templates
        parametrizados. Frases já presentes no <code>campos-padrao.md</code> são filtradas
        automaticamente.
      </p>

      {!vaultPath && (
        <p className="key-error">
          ⚠ Vault não configurado. Configure a pasta nas Configurações antes de aplicar.
        </p>
      )}

      {!templates && !appliedFile && (
        <div className="row">
          <button
            type="button"
            onClick={handleAnalyze}
            disabled={analyzing || !vaultPath}
            title="Pede à IA novos templates de frase recorrentes. Mínimo 5 aprovadas."
          >
            {analyzing ? "Analisando até 80 aprovadas..." : "Buscar frases-modelo"}
          </button>
        </div>
      )}

      {error && <p className="key-error">{error}</p>}

      {appliedFile && (
        <p className="vault-message">
          ✓ Frases-modelo adicionadas em <code>{appliedFile}</code>. Confira no
          Obsidian — os itens vêm sob marker <code>&lt;!-- auto-aprendidos em DATA --&gt;</code>.
        </p>
      )}

      {templates && templates.length === 0 && (
        <p className="help">
          Nenhuma frase-modelo nova emergiu. Aprove mais devolutivas (especialmente
          com texto repetitivo padrão) ou tente novamente.
        </p>
      )}

      {templates && templates.length > 0 && (
        <>
          <div className="selection-toolbar">
            <button
              type="button"
              className="secondary"
              onClick={() => setSelected(new Set(templates.map((_, i) => i)))}
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
              {selected.size}/{templates.length} selecionada(s)
            </span>
          </div>

          <div className="suggestions-list">
            {templates.map((t, i) => (
              <label key={i} className="suggestion-item">
                <input
                  type="checkbox"
                  checked={selected.has(i)}
                  onChange={() => toggleSelected(i)}
                />
                <div className="suggestion-body">
                  <div className="phrase-situation">{t.situation}</div>
                  <div className="phrase-template">{t.template}</div>
                  <div className="suggestion-meta">
                    ~{t.occurrences} ocorrência{t.occurrences === 1 ? "" : "s"} estimada{t.occurrences === 1 ? "" : "s"}
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
              {applying ? "Aplicando..." : `Adicionar ${selected.size} ao campos-padrao.md`}
            </button>
          </div>
        </>
      )}
    </>
  );
}

// ── CartilhaView ──────────────────────────────────────────────────────────────
// Geração de cartilha HTML: dev complementa título + audience + imagens,
// streaming da IA gera o corpo didático, dev edita e salva no vault.

const AUDIENCE_LABELS: Record<string, string> = {
  suporte: "Time de Suporte (N1)",
  cliente: "Usuário final do cliente",
  interno: "Equipe interna",
};

function bytesFromFile(file: File): Promise<Uint8Array> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const buf = reader.result as ArrayBuffer;
      resolve(new Uint8Array(buf));
    };
    reader.onerror = () => reject(reader.error);
    reader.readAsArrayBuffer(file);
  });
}

function previewUrlFromFile(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result as string);
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(file);
  });
}

function extensionFromMime(mime: string): string {
  if (mime === "image/jpeg") return "jpg";
  if (mime === "image/png") return "png";
  if (mime === "image/webp") return "webp";
  if (mime === "image/gif") return "gif";
  if (mime === "image/bmp") return "bmp";
  return "png"; // default seguro
}

async function fileToCartilhaImage(file: File): Promise<CartilhaImage> {
  const [bytes, preview] = await Promise.all([bytesFromFile(file), previewUrlFromFile(file)]);
  return {
    bytes: Array.from(bytes),
    extension: extensionFromMime(file.type),
    caption: "",
    previewUrl: preview,
  };
}

function CartilhaView({
  form,
  release,
  onCancel,
  onSaved,
}: {
  form: FormFields;
  release: string;
  onCancel: () => void;
  onSaved: (path: string) => void;
}) {
  const formInput = useMemo(() => compileForm(form), [form]);
  const imagesGuard = useMemo(() => cartilhaImagesRequired(form), [form]);

  const [titulo, setTitulo] = useState(() => {
    const first = form.correcao.split("\n")[0]?.trim() ?? "";
    return first.length > 80 ? first.slice(0, 80).trimEnd() : first;
  });
  const [autor, setAutor] = useState("");
  const [audience, setAudience] = useState<"suporte" | "cliente" | "interno">("suporte");
  const [images, setImages] = useState<CartilhaImage[]>([]);
  const [dragOver, setDragOver] = useState(false);

  const [aiContent, setAiContent] = useState("");
  const [streaming, setStreaming] = useState(false);
  const [editedContent, setEditedContent] = useState("");
  const [streamingDone, setStreamingDone] = useState(false);

  const [saving, setSaving] = useState(false);
  const [previewing, setPreviewing] = useState(false);
  const [savedPath, setSavedPath] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const aiRef = useRef("");
  const fileInputRef = useRef<HTMLInputElement>(null);

  // Listen tokens de streaming
  useEffect(() => {
    let active = true;
    const ul: UnlistenFn[] = [];
    Promise.all([
      listen<CartilhaTokenEvent>("cartilha-token", (e) => {
        aiRef.current += e.payload.content;
        setAiContent(aiRef.current);
      }),
      listen("cartilha-done", () => {
        setStreaming(false);
        setStreamingDone(true);
        setEditedContent(aiRef.current);
      }),
    ]).then((uls) => {
      if (!active) { uls.forEach((u) => u()); return; }
      ul.push(...uls);
    });
    return () => { active = false; ul.forEach((u) => u()); };
  }, []);

  // Listen paste de imagem (Ctrl+V em qualquer lugar do CartilhaView)
  useEffect(() => {
    const onPaste = async (e: ClipboardEvent) => {
      const items = e.clipboardData?.items;
      if (!items) return;
      const incoming: File[] = [];
      for (let i = 0; i < items.length; i++) {
        const item = items[i];
        if (item.kind === "file" && item.type.startsWith("image/")) {
          const f = item.getAsFile();
          if (f) incoming.push(f);
        }
      }
      if (incoming.length > 0) {
        e.preventDefault();
        const newImgs = await Promise.all(incoming.map(fileToCartilhaImage));
        setImages((prev) => [...prev, ...newImgs]);
      }
    };
    document.addEventListener("paste", onPaste);
    return () => document.removeEventListener("paste", onPaste);
  }, []);

  const handleFilePick = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const files = Array.from(e.target.files ?? []).filter((f) => f.type.startsWith("image/"));
    if (files.length === 0) return;
    const newImgs = await Promise.all(files.map(fileToCartilhaImage));
    setImages((prev) => [...prev, ...newImgs]);
    if (fileInputRef.current) fileInputRef.current.value = "";
  };

  const handleDrop = async (e: React.DragEvent) => {
    e.preventDefault();
    setDragOver(false);
    const files = Array.from(e.dataTransfer.files).filter((f) => f.type.startsWith("image/"));
    if (files.length === 0) return;
    const newImgs = await Promise.all(files.map(fileToCartilhaImage));
    setImages((prev) => [...prev, ...newImgs]);
  };

  const updateCaption = (idx: number, caption: string) => {
    setImages((prev) => prev.map((img, i) => (i === idx ? { ...img, caption } : img)));
  };

  const removeImage = (idx: number) => {
    setImages((prev) => prev.filter((_, i) => i !== idx));
  };

  const canGenerate =
    !!titulo.trim() &&
    !streaming &&
    !streamingDone &&
    (!imagesGuard.required || images.length > 0);

  const handleGenerate = async () => {
    setError(null);
    aiRef.current = "";
    setAiContent("");
    setEditedContent("");
    setStreamingDone(false);
    setStreaming(true);
    try {
      await invoke("stream_cartilha", {
        formInput,
        audience,
        imageCaptions: images.map((img) => img.caption || "(sem legenda)"),
      });
    } catch (e) {
      setError(`Erro ao gerar: ${String(e)}`);
      setStreaming(false);
    }
  };

  const handleSave = async () => {
    if (!editedContent.trim()) return;
    setSaving(true);
    setError(null);
    try {
      const path = await invoke<string>("save_cartilha", {
        title: titulo.trim(),
        content: editedContent,
        release: release.trim() || null,
        author: autor.trim() || null,
        images: images.map(({ bytes, extension, caption }) => ({ bytes, extension, caption })),
      });
      setSavedPath(path);
      onSaved(path);
    } catch (e) {
      setError(`Erro ao salvar: ${String(e)}`);
    } finally {
      setSaving(false);
    }
  };

  const handleOpen = async () => {
    if (!savedPath) return;
    try {
      await invoke("open_in_system", { path: savedPath });
    } catch (e) {
      setError(`Erro ao abrir: ${String(e)}`);
    }
  };

  const handlePreview = async () => {
    if (!editedContent.trim()) return;
    setPreviewing(true);
    setError(null);
    try {
      const path = await invoke<string>("preview_cartilha", {
        title: titulo.trim(),
        content: editedContent,
        release: release.trim() || null,
        author: autor.trim() || null,
        images: images.map(({ bytes, extension, caption }) => ({ bytes, extension, caption })),
      });
      await invoke("open_in_system", { path });
    } catch (e) {
      setError(`Erro ao pré-visualizar: ${String(e)}`);
    } finally {
      setPreviewing(false);
    }
  };

  return (
    <div className="cartilha-view">
      <header className="cartilha-header">
        <h2>Cartilha HTML</h2>
        <button className="secondary close" onClick={onCancel} aria-label="Voltar">✕</button>
      </header>

      {savedPath ? (
        <div className="cartilha-success">
          <p className="vault-message">
            ✓ Cartilha salva em <code>{savedPath}</code>
          </p>
          <div className="row">
            <button type="button" className="btn-secondary" onClick={onCancel}>
              Nova devolutiva
            </button>
            <button type="button" onClick={handleOpen}>
              Abrir no navegador
            </button>
          </div>
        </div>
      ) : (
        <>
          <div className="cartilha-meta">
            <div className="form-field">
              <label>Título da cartilha</label>
              <input
                type="text"
                value={titulo}
                onChange={(e) => setTitulo(e.target.value)}
                placeholder="Ex: Nova permissão de cadastro de produto"
                disabled={streaming || saving}
              />
            </div>
            <div className="form-row-2">
              <div className="form-field">
                <label>Autor <span className="field-optional">opcional</span></label>
                <input
                  type="text"
                  value={autor}
                  onChange={(e) => setAutor(e.target.value)}
                  placeholder="Seu nome"
                  disabled={streaming || saving}
                />
              </div>
              <div className="form-field">
                <label>Público alvo</label>
                <select
                  value={audience}
                  onChange={(e) => setAudience(e.target.value as "suporte" | "cliente" | "interno")}
                  disabled={streaming || saving}
                >
                  <option value="suporte">{AUDIENCE_LABELS.suporte}</option>
                  <option value="cliente">{AUDIENCE_LABELS.cliente}</option>
                  <option value="interno">{AUDIENCE_LABELS.interno}</option>
                </select>
              </div>
            </div>
          </div>

          <div className="cartilha-images-section">
            <div className="cartilha-images-header">
              <span className="label">
                Imagens
                {imagesGuard.required && (
                  <span className="field-required" title={imagesGuard.reason ?? ""}> obrigatórias</span>
                )}
              </span>
              <span className="counter">{images.length} anexada(s)</span>
            </div>
            {imagesGuard.required && images.length === 0 && (
              <p className="key-error">
                ⚠ Imagens obrigatórias porque {imagesGuard.reason}.
              </p>
            )}
            <p className="help" style={{ marginTop: 4 }}>
              Cole (<code>Ctrl+V</code>), arraste arquivos aqui ou clique no botão. PNG, JPG, WEBP, GIF, BMP.
            </p>
            <div
              className={`cartilha-dropzone ${dragOver ? "drag-over" : ""}`}
              onDragOver={(e) => { e.preventDefault(); setDragOver(true); }}
              onDragLeave={() => setDragOver(false)}
              onDrop={handleDrop}
            >
              {images.length === 0 ? (
                <span className="dropzone-empty">Solte imagens aqui ou cole com Ctrl+V</span>
              ) : (
                <div className="cartilha-images-grid">
                  {images.map((img, i) => (
                    <div key={i} className="cartilha-image-card">
                      <img src={img.previewUrl} alt={`Imagem ${i + 1}`} />
                      <input
                        type="text"
                        value={img.caption}
                        onChange={(e) => updateCaption(i, e.target.value)}
                        placeholder="Legenda da imagem"
                        disabled={streaming || saving}
                      />
                      <button
                        type="button"
                        className="btn-danger"
                        onClick={() => removeImage(i)}
                        disabled={streaming || saving}
                      >
                        Remover
                      </button>
                    </div>
                  ))}
                </div>
              )}
            </div>
            <input
              ref={fileInputRef}
              type="file"
              accept="image/*"
              multiple
              style={{ display: "none" }}
              onChange={handleFilePick}
            />
            <div className="row">
              <button
                type="button"
                className="btn-secondary"
                onClick={() => fileInputRef.current?.click()}
                disabled={streaming || saving}
              >
                Anexar arquivo...
              </button>
            </div>
          </div>

          <div className="cartilha-content-section">
            {!streamingDone && !streaming && (
              <div className="row">
                <button
                  type="button"
                  onClick={handleGenerate}
                  disabled={!canGenerate}
                  title="A IA escreve o conteúdo a partir do input do formulário"
                >
                  Gerar conteúdo
                </button>
              </div>
            )}
            {streaming && (
              <div className="result-waiting">
                Gerando cartilha<span className="result-dots" />
              </div>
            )}
            {streaming && aiContent && (
              <pre className="cartilha-streaming">{aiContent}</pre>
            )}
            {streamingDone && (
              <>
                <p className="help">
                  Revise e edite antes de salvar. Use <code>[s]Título :[/s]</code> para
                  seções, <code>[img 1]</code> em linha própria para posicionar imagens e{" "}
                  <code>[dica]</code>/<code>[atencao]</code>/<code>[ok]</code> para caixas
                  de destaque.
                </p>
                <textarea
                  className="cartilha-editor"
                  value={editedContent}
                  onChange={(e) => setEditedContent(e.target.value)}
                  disabled={saving}
                  spellCheck
                />
                <div className="row">
                  <button
                    type="button"
                    className="btn-secondary"
                    onClick={() => {
                      aiRef.current = "";
                      setAiContent("");
                      setEditedContent("");
                      setStreamingDone(false);
                    }}
                    disabled={saving || previewing}
                  >
                    Gerar novamente
                  </button>
                  <button
                    type="button"
                    className="btn-secondary"
                    onClick={handlePreview}
                    disabled={saving || previewing || !editedContent.trim()}
                    title="Renderiza o HTML numa pasta temporária e abre no navegador (não salva no vault)"
                  >
                    {previewing ? "Abrindo..." : "Pré-visualizar"}
                  </button>
                  <button
                    type="button"
                    onClick={handleSave}
                    disabled={saving || previewing || !editedContent.trim()}
                  >
                    {saving ? "Salvando..." : "Salvar no vault"}
                  </button>
                </div>
              </>
            )}
          </div>
        </>
      )}

      {error && <p className="key-error">{error}</p>}
    </div>
  );
}

// ── TestesView ────────────────────────────────────────────────────────────────
// Form complementar pro QA: dev preenche 6 seções (algumas pré-preenchidas do
// FormView), opcionalmente pede sugestões da IA, e clica "Gerar e copiar" pra
// receber texto estruturado pronto pro ticket de QA.

interface TestesFields {
  ticket: string;
  dataEntrega: string;
  tipo: string;
  ambiente: string;
  scripts: string;
  parametros: string;
  dadosTeste: string;
  happyPath: string;
  edgeCases: string;
  negativeCases: string;
  acceptanceCriteria: string;
  regressionAreas: string;
  risks: string;
}

function TestesView({
  form,
  release,
  onCancel,
  onCopied,
}: {
  form: FormFields;
  release: string;
  onCancel: () => void;
  onCopied: () => void;
}) {
  const formInput = useMemo(() => compileForm(form), [form]);

  const [fields, setFields] = useState<TestesFields>(() => ({
    ticket: "",
    dataEntrega: new Date().toISOString().slice(0, 10),
    tipo: "correcao",
    ambiente: "homologacao",
    scripts: form.scripts,
    parametros: form.parametro,
    dadosTeste: "",
    happyPath: form.validacao,
    edgeCases: "",
    negativeCases: "",
    acceptanceCriteria: "",
    regressionAreas: "",
    risks: form.cenario,
  }));

  const [suggesting, setSuggesting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const set = <K extends keyof TestesFields>(key: K) =>
    (e: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement>) =>
      setFields((prev) => ({ ...prev, [key]: e.target.value }));

  const handleSuggest = async () => {
    setSuggesting(true);
    setError(null);
    try {
      const sug = await invoke<TestScenariosSuggestion>("suggest_test_scenarios", { formInput });
      setFields((prev) => ({
        ...prev,
        happyPath: sug.happy_path || prev.happyPath,
        edgeCases: sug.edge_cases || prev.edgeCases,
        negativeCases: sug.negative_cases || prev.negativeCases,
        acceptanceCriteria: sug.acceptance_criteria || prev.acceptanceCriteria,
        regressionAreas: sug.regression_areas || prev.regressionAreas,
        risks: sug.risks || prev.risks,
      }));
    } catch (e) {
      setError(String(e));
    } finally {
      setSuggesting(false);
    }
  };

  const tipoLabels: Record<string, string> = {
    correcao: "Correção",
    feature: "Nova feature",
    melhoria: "Melhoria",
  };
  const ambienteLabels: Record<string, string> = {
    homologacao: "Homologação",
    espelho: "Produção espelho",
    dev: "Desenvolvimento",
  };

  const compileTestesText = (): string => {
    const parts: string[] = [];
    const section = (title: string, body: string) => {
      if (body.trim()) parts.push(`[n]${title} :[/n]\n\n${body.trim()}`);
    };
    const line = (label: string, value: string) => (value.trim() ? `${label}: ${value.trim()}` : null);

    const idLines = [
      line("Ticket", fields.ticket),
      line("Release", release),
      line("Data de entrega para teste", fields.dataEntrega),
      line("Tipo", tipoLabels[fields.tipo] ?? fields.tipo),
    ].filter(Boolean) as string[];
    if (idLines.length > 0) parts.push(`[n]Identificação :[/n]\n\n${idLines.join("\n")}`);

    section("Contexto", form.correcao);

    const preReqLines: string[] = [];
    if (fields.ambiente.trim()) preReqLines.push(`Ambiente: ${ambienteLabels[fields.ambiente] ?? fields.ambiente}`);
    if (fields.scripts.trim()) preReqLines.push(`Scripts a rodar antes:\n\`\`\`sql\n${fields.scripts.trim()}\n\`\`\``);
    if (fields.parametros.trim()) preReqLines.push(`Parâmetros a setar:\n${fields.parametros.trim()}`);
    if (fields.dadosTeste.trim()) preReqLines.push(`Dados de teste:\n${fields.dadosTeste.trim()}`);
    if (preReqLines.length > 0) parts.push(`[n]Pré-requisitos :[/n]\n\n${preReqLines.join("\n\n")}`);

    const cenarioLines: string[] = [];
    if (fields.happyPath.trim()) cenarioLines.push(`Caminho feliz:\n${fields.happyPath.trim()}`);
    if (fields.edgeCases.trim()) cenarioLines.push(`Cenários edge:\n${fields.edgeCases.trim()}`);
    if (fields.negativeCases.trim()) cenarioLines.push(`Cenários negativos:\n${fields.negativeCases.trim()}`);
    if (fields.acceptanceCriteria.trim()) cenarioLines.push(`Critérios de aceitação:\n${fields.acceptanceCriteria.trim()}`);
    if (cenarioLines.length > 0) parts.push(`[n]Cenários :[/n]\n\n${cenarioLines.join("\n\n")}`);

    section("Regressão", fields.regressionAreas);
    section("Riscos e observações", fields.risks);

    return parts.join("\n\n");
  };

  const handleCopy = async () => {
    const text = compileTestesText();
    if (!text.trim()) {
      setError("Preencha ao menos uma seção antes de copiar.");
      return;
    }
    setError(null);
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      onCopied();
      setTimeout(() => setCopied(false), 2500);
    } catch (e) {
      setError(`Erro ao copiar: ${String(e)}`);
    }
  };

  return (
    <div className="testes-view">
      <header className="cartilha-header">
        <h2>Form de testes (QA)</h2>
        <button className="secondary close" onClick={onCancel} aria-label="Voltar">✕</button>
      </header>

      <p className="help">
        Campos pré-preenchidos vêm do formulário principal. Clique em "Sugerir cenários" para a IA propor cenários, edge cases e regressão a partir do contexto.
      </p>

      <div className="testes-form">
        <h3 className="testes-section-title">1. Identificação</h3>
        <div className="form-row-2">
          <div className="form-field">
            <label>Ticket / chamado</label>
            <input type="text" value={fields.ticket} onChange={set("ticket")} placeholder="ART-1234" />
          </div>
          <div className="form-field">
            <label>Data de entrega para teste</label>
            <input type="date" value={fields.dataEntrega} onChange={set("dataEntrega")} />
          </div>
        </div>
        <div className="form-field">
          <label>Tipo</label>
          <select value={fields.tipo} onChange={set("tipo")}>
            <option value="correcao">Correção</option>
            <option value="feature">Nova feature</option>
            <option value="melhoria">Melhoria</option>
          </select>
        </div>

        <h3 className="testes-section-title">2. Pré-requisitos</h3>
        <div className="form-field">
          <label>Ambiente</label>
          <select value={fields.ambiente} onChange={set("ambiente")}>
            <option value="homologacao">Homologação</option>
            <option value="espelho">Produção espelho</option>
            <option value="dev">Desenvolvimento</option>
          </select>
        </div>
        <div className="form-field">
          <label>Scripts a rodar antes <span className="field-optional">prefill</span></label>
          <textarea rows={3} value={fields.scripts} onChange={set("scripts")} className="monospace" />
        </div>
        <div className="form-field">
          <label>Parâmetros a setar <span className="field-optional">prefill</span></label>
          <textarea rows={2} value={fields.parametros} onChange={set("parametros")} />
        </div>
        <div className="form-field">
          <label>Dados de teste <span className="field-optional">opcional</span></label>
          <textarea
            rows={2}
            value={fields.dadosTeste}
            onChange={set("dadosTeste")}
            placeholder="Usuário, empresa, valores, casos específicos..."
          />
        </div>

        <h3 className="testes-section-title">
          3. Cenários
          <button
            type="button"
            className="btn-secondary suggest-ai"
            onClick={handleSuggest}
            disabled={suggesting}
            title="A IA preenche cenários, regressão e riscos a partir do contexto"
          >
            {suggesting ? "Sugerindo..." : "Sugerir com IA"}
          </button>
        </h3>
        <div className="form-field">
          <label>Caminho feliz</label>
          <textarea rows={3} value={fields.happyPath} onChange={set("happyPath")} />
        </div>
        <div className="form-field">
          <label>Cenários edge (limites, valores extremos)</label>
          <textarea rows={3} value={fields.edgeCases} onChange={set("edgeCases")} />
        </div>
        <div className="form-field">
          <label>Cenários negativos (que devem bloquear)</label>
          <textarea rows={3} value={fields.negativeCases} onChange={set("negativeCases")} />
        </div>
        <div className="form-field">
          <label>Critérios de aceitação</label>
          <textarea rows={3} value={fields.acceptanceCriteria} onChange={set("acceptanceCriteria")} />
        </div>

        <h3 className="testes-section-title">4. Regressão</h3>
        <div className="form-field">
          <label>Áreas afetadas que devem ser revalidadas</label>
          <textarea rows={3} value={fields.regressionAreas} onChange={set("regressionAreas")} />
        </div>

        <h3 className="testes-section-title">5. Riscos e observações</h3>
        <div className="form-field">
          <label>Limitações conhecidas, cenários não simulados</label>
          <textarea rows={3} value={fields.risks} onChange={set("risks")} />
        </div>
      </div>

      {error && <p className="key-error">{error}</p>}

      <div className="form-footer form-footer-multi">
        <button type="button" className="btn-secondary" onClick={onCancel}>
          Voltar
        </button>
        <button type="button" onClick={handleCopy} disabled={suggesting}>
          {copied ? "✓ Copiado!" : "Gerar e copiar"}
        </button>
      </div>
    </div>
  );
}

// ── ChatWindow (main) ─────────────────────────────────────────────────────────

export default function ChatWindow() {
  const [apiKey, setApiKey] = useState<string | null>(null);
  const [keyLoaded, setKeyLoaded] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [showLearning, setShowLearning] = useState(false);
  const [vaultStatus, setVaultStatus] = useState<VaultStatus | null>(null);

  const [view, setView] = useState<"form" | "result" | "cartilha" | "testes">("form");
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

      {view === "form" && (
        <FormView
          fields={form}
          onChange={setForm}
          onGenerate={handleGenerate}
          onGenerateCartilha={() => setView("cartilha")}
          onGenerateTestes={() => setView("testes")}
          disabled={streaming}
        />
      )}
      {view === "result" && (
        <ResultView
          aiRawOutput={result}
          streaming={streaming}
          category={currentCategory}
          onNew={handleNewDevolutiva}
          onApprove={handleApprove}
          onDiscard={handleDiscard}
        />
      )}
      {view === "cartilha" && (
        <CartilhaView
          form={form}
          release={form.release}
          onCancel={() => setView("form")}
          onSaved={() => { /* mantém na tela de sucesso até usuário voltar */ }}
        />
      )}
      {view === "testes" && (
        <TestesView
          form={form}
          release={form.release}
          onCancel={() => setView("form")}
          onCopied={() => { /* toast já está dentro do componente */ }}
        />
      )}
    </div>
  );
}
