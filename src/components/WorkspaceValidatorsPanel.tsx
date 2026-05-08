import { useState, useEffect, useCallback } from "react";
import { api } from "../lib/api";
import type { WorkspaceValidator, ValidatorTypeInfo } from "../types";
import type { PanesError } from "../types/errors";

interface Props {
  workspaceId: string;
  workspaceName?: string;
}

function errorMessage(e: unknown): string {
  if (!e) return "Unknown error";
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  if (typeof e === "object" && "message" in e) {
    const m = (e as PanesError).message;
    if (typeof m === "string" && m.length > 0) return m;
  }
  try {
    return JSON.stringify(e);
  } catch {
    return String(e);
  }
}

function safeParseJson(s: string): Record<string, unknown> {
  try {
    const parsed = JSON.parse(s);
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
      return parsed as Record<string, unknown>;
    }
  } catch {
    // fall through
  }
  return {};
}

function stringifyJson(value: unknown): string {
  try {
    return JSON.stringify(value ?? {}, null, 2);
  } catch {
    return "{}";
  }
}

export default function WorkspaceValidatorsPanel({
  workspaceId,
  workspaceName,
}: Props) {
  const [types, setTypes] = useState<ValidatorTypeInfo[]>([]);
  const [rows, setRows] = useState<WorkspaceValidator[]>([]);
  const [loading, setLoading] = useState(true);
  const [err, setErr] = useState<string | null>(null);
  const [addingType, setAddingType] = useState<string | null>(null);
  const [expandedId, setExpandedId] = useState<string | null>(null);

  const load = useCallback(async () => {
    setErr(null);
    try {
      const [t, r] = await Promise.all([
        api.listValidatorTypes(),
        api.listValidators(workspaceId),
      ]);
      setTypes(t);
      setRows(r);
    } catch (e) {
      setErr(errorMessage(e));
    } finally {
      setLoading(false);
    }
  }, [workspaceId]);

  useEffect(() => {
    setLoading(true);
    void load();
  }, [load]);

  const addForType = async (typeId: string) => {
    const typeInfo = types.find((t) => t.typeId === typeId);
    const defaultCfg = typeInfo?.defaultConfig ?? {};
    try {
      await api.addValidator({
        workspaceId,
        validatorType: typeId,
        configJson: JSON.stringify(defaultCfg),
      });
      setAddingType(null);
      await load();
    } catch (e) {
      setErr(errorMessage(e));
    }
  };

  const toggleEnabled = async (v: WorkspaceValidator, enabled: boolean) => {
    setErr(null);
    try {
      const updated = await api.updateValidator({ id: v.id, enabled });
      setRows((prev) => prev.map((r) => (r.id === v.id ? updated : r)));
    } catch (e) {
      setErr(errorMessage(e));
    }
  };

  const updateConfig = async (
    v: WorkspaceValidator,
    nextConfig: Record<string, unknown>,
  ) => {
    setErr(null);
    try {
      const updated = await api.updateValidator({
        id: v.id,
        configJson: JSON.stringify(nextConfig),
      });
      setRows((prev) => prev.map((r) => (r.id === v.id ? updated : r)));
    } catch (e) {
      setErr(errorMessage(e));
    }
  };

  const removeRow = async (id: string) => {
    setErr(null);
    try {
      await api.removeValidator(id);
      setRows((prev) => prev.filter((r) => r.id !== id));
      if (expandedId === id) setExpandedId(null);
    } catch (e) {
      setErr(errorMessage(e));
    }
  };

  const resolveTypeInfo = (typeId: string): ValidatorTypeInfo | undefined =>
    types.find((t) => t.typeId === typeId);

  const availableTypes = types.filter(
    (t) => !rows.some((r) => r.validatorType === t.typeId),
  );

  if (loading) {
    return (
      <div className="panel-loading">
        <span className="spinner" />
      </div>
    );
  }

  return (
    <div className="validators-panel">
      <div className="validators-panel-header">
        <div>
          <h2>Output Validators</h2>
          <p className="validators-panel-subtitle">
            Validators are scoped to{" "}
            <strong>{workspaceName ?? "this workspace"}</strong>. They run after
            every completion in this workspace; failures pause the thread and
            prompt you to accept or reject the output.
          </p>
        </div>
      </div>

      {err && (
        <div className="validators-error">
          <span className="validators-error-icon">!</span>
          {err}
        </div>
      )}

      {rows.length === 0 && availableTypes.length === 0 && (
        <div className="validators-empty">
          <p>No validators available.</p>
        </div>
      )}

      {rows.length > 0 && (
        <div className="validators-list">
          {rows.map((v) => {
            const info = resolveTypeInfo(v.validatorType);
            const expanded = expandedId === v.id;
            return (
              <div
                key={v.id}
                className={`validators-item ${!v.enabled ? "disabled" : ""}`}
              >
                <div className="validators-item-header">
                  <div
                    className="validators-item-main"
                    onClick={() => setExpandedId(expanded ? null : v.id)}
                  >
                    <div className="validators-title-row">
                      <span className="validators-label">
                        {info?.label ?? v.validatorType}
                      </span>
                      {!v.enabled && (
                        <span className="validators-disabled-badge">disabled</span>
                      )}
                    </div>
                    {info && (
                      <span className="validators-description">
                        {info.description}
                      </span>
                    )}
                  </div>
                  <div className="validators-item-actions">
                    <label
                      className="toggle"
                      onClick={(e) => e.stopPropagation()}
                      title={v.enabled ? "Disable" : "Enable"}
                    >
                      <input
                        type="checkbox"
                        checked={v.enabled}
                        onChange={(e) => toggleEnabled(v, e.target.checked)}
                      />
                      <span className="toggle-slider" />
                    </label>
                    <button
                      className="btn-icon btn-delete-inline"
                      onClick={(e) => {
                        e.stopPropagation();
                        removeRow(v.id);
                      }}
                      title="Remove validator"
                    >
                      <svg
                        width="12"
                        height="12"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        strokeWidth="2"
                        strokeLinecap="round"
                        strokeLinejoin="round"
                      >
                        <path d="M3 6h18" />
                        <path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
                        <path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6" />
                      </svg>
                    </button>
                  </div>
                </div>

                {expanded && (
                  <ValidatorConfigEditor
                    validator={v}
                    onChange={(next) => updateConfig(v, next)}
                  />
                )}
              </div>
            );
          })}
        </div>
      )}

      {availableTypes.length > 0 && (
        <div className="validators-add-section">
          <div className="validators-add-label">Add a validator</div>
          <div className="validators-add-options">
            {availableTypes.map((t) => (
              <button
                key={t.typeId}
                className="validators-add-option"
                onClick={() => {
                  setAddingType(t.typeId);
                  void addForType(t.typeId);
                }}
                disabled={addingType === t.typeId}
              >
                <span className="validators-add-option-label">{t.label}</span>
                <span className="validators-add-option-desc">
                  {t.description}
                </span>
              </button>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

interface ConfigEditorProps {
  validator: WorkspaceValidator;
  onChange: (next: Record<string, unknown>) => void;
}

function ValidatorConfigEditor({ validator, onChange }: ConfigEditorProps) {
  switch (validator.validatorType) {
    case "citation":
      return <CitationConfigEditor validator={validator} onChange={onChange} />;
    case "secret_scan":
      return (
        <SecretScanConfigEditor validator={validator} onChange={onChange} />
      );
    default:
      return <RawJsonEditor validator={validator} onChange={onChange} />;
  }
}

function CitationConfigEditor({ validator, onChange }: ConfigEditorProps) {
  const cfg = safeParseJson(validator.configJson);
  const checkLineRefs = (cfg.check_line_refs as boolean | undefined) ?? true;
  const allowOutside =
    (cfg.allow_outside_workspace as boolean | undefined) ?? false;
  const [advanced, setAdvanced] = useState(false);

  return (
    <div className="validators-config">
      <label className="validators-config-row">
        <input
          type="checkbox"
          checked={checkLineRefs}
          onChange={(e) =>
            onChange({ ...cfg, check_line_refs: e.target.checked })
          }
        />
        <div>
          <span className="validators-config-label">
            Verify line references
          </span>
          <span className="validators-config-hint">
            When the output mentions <code>path:NN</code>, flag if NN is beyond
            the file's line count.
          </span>
        </div>
      </label>
      <label className="validators-config-row">
        <input
          type="checkbox"
          checked={allowOutside}
          onChange={(e) =>
            onChange({ ...cfg, allow_outside_workspace: e.target.checked })
          }
        />
        <div>
          <span className="validators-config-label">
            Allow paths outside the workspace
          </span>
          <span className="validators-config-hint">
            By default, absolute paths outside this workspace produce a
            warning. Enable this to accept them silently.
          </span>
        </div>
      </label>
      <AdvancedToggle advanced={advanced} setAdvanced={setAdvanced} />
      {advanced && <RawJsonEditor validator={validator} onChange={onChange} />}
    </div>
  );
}

function SecretScanConfigEditor({ validator, onChange }: ConfigEditorProps) {
  const cfg = safeParseJson(validator.configJson);
  const patterns = Array.isArray(cfg.custom_patterns)
    ? (cfg.custom_patterns as unknown[]).filter(
        (p): p is string => typeof p === "string",
      )
    : [];
  const [draft, setDraft] = useState("");
  const [advanced, setAdvanced] = useState(false);

  const addPattern = () => {
    if (!draft.trim()) return;
    onChange({ ...cfg, custom_patterns: [...patterns, draft.trim()] });
    setDraft("");
  };

  const removePattern = (i: number) => {
    const next = patterns.filter((_, idx) => idx !== i);
    onChange({ ...cfg, custom_patterns: next });
  };

  return (
    <div className="validators-config">
      <div className="validators-config-label">Custom regex patterns</div>
      <div className="validators-config-hint">
        Built-in patterns (AWS keys, GitHub tokens, private keys, Slack tokens)
        always run. Add extra regex rules specific to this workspace.
      </div>

      {patterns.length > 0 && (
        <ul className="validators-pattern-list">
          {patterns.map((p, i) => (
            <li key={i}>
              <code>{p}</code>
              <button
                className="btn btn-sm btn-secondary"
                onClick={() => removePattern(i)}
              >
                Remove
              </button>
            </li>
          ))}
        </ul>
      )}

      <div className="validators-pattern-add">
        <input
          type="text"
          placeholder="e.g. INTERNAL-[A-Z]{6}-[0-9]{4}"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              addPattern();
            }
          }}
        />
        <button
          className="btn btn-sm btn-primary"
          onClick={addPattern}
          disabled={!draft.trim()}
        >
          Add
        </button>
      </div>

      <AdvancedToggle advanced={advanced} setAdvanced={setAdvanced} />
      {advanced && <RawJsonEditor validator={validator} onChange={onChange} />}
    </div>
  );
}

function AdvancedToggle({
  advanced,
  setAdvanced,
}: {
  advanced: boolean;
  setAdvanced: (v: boolean) => void;
}) {
  return (
    <button
      className="validators-advanced-toggle"
      onClick={() => setAdvanced(!advanced)}
    >
      {advanced ? "Hide advanced (JSON)" : "Show advanced (JSON)"}
    </button>
  );
}

function RawJsonEditor({ validator, onChange }: ConfigEditorProps) {
  const [draft, setDraft] = useState(() =>
    stringifyJson(safeParseJson(validator.configJson)),
  );
  const [parseErr, setParseErr] = useState<string | null>(null);

  useEffect(() => {
    setDraft(stringifyJson(safeParseJson(validator.configJson)));
  }, [validator.configJson]);

  const save = () => {
    try {
      const parsed = JSON.parse(draft);
      if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
        setParseErr("Config must be a JSON object");
        return;
      }
      setParseErr(null);
      onChange(parsed as Record<string, unknown>);
    } catch (e) {
      setParseErr(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <div className="validators-raw-json">
      <textarea
        rows={6}
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        spellCheck={false}
      />
      {parseErr && <div className="validators-error-inline">{parseErr}</div>}
      <button className="btn btn-sm btn-primary" onClick={save}>
        Save JSON
      </button>
    </div>
  );
}
