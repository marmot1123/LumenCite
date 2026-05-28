/**
 * AuthorEditor — 著者 1 件を編集するモーダル。
 *
 * v0.3.0 で導入された多言語名・読み仮名・国際識別子・団体著者フラグまで全フィールドを編集できる。
 * Tauri コマンド `get_author` / `update_author` / `search_authors` / `merge_authors` を使う。
 *
 * 保存すると backend 側で当該著者を含む全文献の entries_fts が再同期されるため、
 * 検索結果に名前・読み仮名・原語表記が即反映される（plan §3.4）。
 */
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import type { Author, AuthorIdentifierInput, AuthorInput } from "../types";

interface AuthorEditorProps {
  authorId: number;
  onClose: () => void;
  /** 保存または統合が成功したときに呼ばれる。呼び出し側で一覧再描画などに使う。 */
  onSaved?: (updated: Author | null) => void;
}

const SCRIPT_OPTIONS = ["Latn", "Hani", "Hang", "Cyrl", "Arab", "Hira", "Kana"] as const;
const SCHEME_OPTIONS = [
  "orcid",
  "scopus",
  "dblp",
  "semantic_scholar",
  "wikidata",
  "isni",
  "viaf",
  "researcher_id",
  "google_scholar",
] as const;

const ORCID_PATTERN = /^\d{4}-\d{4}-\d{4}-\d{3}[\dX]$/;
const URL_PATTERN = /^https?:\/\//i;

function authorToInput(a: Author): AuthorInput {
  return {
    name: a.name,
    given_name: a.given_name ?? null,
    middle_name: a.middle_name ?? null,
    family_name: a.family_name ?? null,
    suffix: a.suffix ?? null,
    name_particle: a.name_particle ?? null,
    name_original: a.name_original ?? null,
    given_name_original: a.given_name_original ?? null,
    family_name_original: a.family_name_original ?? null,
    original_script: a.original_script ?? null,
    reading_family: a.reading_family ?? null,
    reading_given: a.reading_given ?? null,
    is_organization: a.is_organization,
    email: a.email ?? null,
    homepage_url: a.homepage_url ?? null,
    notes: a.notes ?? null,
    orcid: a.orcid ?? null,
    identifiers: a.identifiers
      .filter(i => i.scheme !== "orcid") // orcid は専用フィールドに分離
      .map(i => ({ scheme: i.scheme, value: i.value, url: i.url ?? null })),
  };
}

function trimOrNull(s: string | null | undefined): string | null {
  if (s == null) return null;
  const t = s.trim();
  return t === "" ? null : t;
}

export function AuthorEditor({ authorId, onClose, onSaved }: AuthorEditorProps) {
  const { t } = useTranslation();
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [merging, setMerging] = useState(false);
  const [fetching, setFetching] = useState(false);
  const [fetchSummary, setFetchSummary] = useState<string | null>(null);
  const [form, setForm] = useState<AuthorInput | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [mergeQuery, setMergeQuery] = useState("");
  const [mergeCandidates, setMergeCandidates] = useState<Author[]>([]);
  const [confirmMergeFrom, setConfirmMergeFrom] = useState<Author | null>(null);

  // 初期ロード
  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    invoke<Author | null>("get_author", { id: authorId })
      .then(a => {
        if (cancelled) return;
        if (a) setForm(authorToInput(a));
        else setError("Author not found");
      })
      .catch(e => !cancelled && setError(String(e)))
      .finally(() => !cancelled && setLoading(false));
    return () => { cancelled = true; };
  }, [authorId]);

  // 統合候補の検索（簡易デバウンス）
  useEffect(() => {
    const q = mergeQuery.trim();
    if (q.length < 2) { setMergeCandidates([]); return; }
    let cancelled = false;
    const handle = setTimeout(() => {
      invoke<Author[]>("search_authors", { query: q, limit: 8 })
        .then(rows => {
          if (cancelled) return;
          // 自分自身は候補から除外
          setMergeCandidates(rows.filter(r => r.id !== authorId));
        })
        .catch(() => { /* noop */ });
    }, 200);
    return () => { cancelled = true; clearTimeout(handle); };
  }, [mergeQuery, authorId]);

  const validationKey = useMemo(() => validate(form), [form]);
  const validationError = validationKey ? t(validationKey) : null;

  const canSave = !!form && !saving && !loading && !validationError && !merging;

  const handleSave = async () => {
    if (!form || !canSave) return;
    setSaving(true);
    setError(null);
    try {
      // 空文字 → null に揃え、空 identifiers 行は捨ててから送る
      const normalized: AuthorInput = {
        ...form,
        name: form.name.trim(),
        given_name: trimOrNull(form.given_name),
        middle_name: trimOrNull(form.middle_name),
        family_name: trimOrNull(form.family_name),
        suffix: trimOrNull(form.suffix),
        name_particle: trimOrNull(form.name_particle),
        name_original: trimOrNull(form.name_original),
        given_name_original: trimOrNull(form.given_name_original),
        family_name_original: trimOrNull(form.family_name_original),
        original_script: trimOrNull(form.original_script),
        reading_family: trimOrNull(form.reading_family),
        reading_given: trimOrNull(form.reading_given),
        email: trimOrNull(form.email),
        homepage_url: trimOrNull(form.homepage_url),
        notes: trimOrNull(form.notes),
        orcid: trimOrNull(form.orcid),
        identifiers: (form.identifiers ?? [])
          .map(i => ({
            scheme: i.scheme.trim(),
            value: i.value.trim(),
            url: trimOrNull(i.url),
          }))
          .filter(i => i.scheme !== "" && i.value !== ""),
      };
      const updated = await invoke<Author>("update_author", { id: authorId, input: normalized });
      onSaved?.(updated);
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  // ORCID Public API から不足フィールドを引いてくる。既存入力は温存。
  const handleFetchFromOrcid = async () => {
    if (!form) return;
    const orcid = (form.orcid ?? "").trim();
    if (!ORCID_PATTERN.test(orcid)) {
      setFetchSummary(null);
      setError(t("authorEditor.fetch.invalidOrcid"));
      return;
    }
    setFetching(true);
    setError(null);
    setFetchSummary(null);
    try {
      const fetched = await invoke<AuthorInput>("fetch_author_from_orcid", { orcid });
      const merged = mergeFetchedIntoForm(form, fetched);
      setForm(merged.next);
      setFetchSummary(
        merged.fieldsFilled === 0 && merged.identifiersAdded === 0
          ? t("authorEditor.fetch.noChanges")
          : t("authorEditor.fetch.summary", {
              fields: merged.fieldsFilled,
              identifiers: merged.identifiersAdded,
            }),
      );
    } catch (e) {
      setError(String(e));
    } finally {
      setFetching(false);
    }
  };

  const handleMerge = async () => {
    if (!confirmMergeFrom) return;
    setMerging(true);
    setError(null);
    try {
      // confirmMergeFrom → 現在の著者 へ統合（fromId / intoId は Rust 側の引数名）
      await invoke("merge_authors", { fromId: confirmMergeFrom.id, intoId: authorId });
      onSaved?.(null);
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setMerging(false);
    }
  };

  return (
    <div
      onClick={onClose}
      style={{
        position: "fixed", inset: 0,
        background: "rgba(0,0,0,0.30)",
        display: "flex", alignItems: "center", justifyContent: "center",
        zIndex: 1000,
      }}
    >
      <div
        onClick={e => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
        style={{
          width: 640, maxWidth: "94vw", maxHeight: "92vh",
          background: "var(--surface)",
          border: "1px solid var(--border-strong)",
          borderRadius: 10,
          boxShadow: "0 12px 32px rgba(0,0,0,0.18)",
          display: "flex", flexDirection: "column",
        }}
      >
        <header style={{
          padding: "16px 22px 12px",
          borderBottom: "1px solid var(--border)",
          display: "flex", alignItems: "baseline", justifyContent: "space-between",
        }}>
          <div style={{ fontSize: 15, fontWeight: 600, color: "var(--text)" }}>
            {t("authorEditor.title")}
          </div>
          <div style={{ fontSize: 11, color: "var(--text-faint)" }}>
            {t("authorEditor.saveHelp")}
          </div>
        </header>

        <div style={{
          padding: "16px 22px",
          overflowY: "auto",
          flex: 1,
        }}>
          {loading && <div style={hintStyle}>…</div>}
          {!loading && form && (
            <>
              <Section title={t("authorEditor.section.name")}>
                <Row>
                  <Field label={t("authorEditor.field.displayName")} required>
                    <input
                      value={form.name}
                      onChange={e => setForm({ ...form, name: e.target.value })}
                      style={inputStyle}
                    />
                  </Field>
                </Row>
                <div style={hintStyle}>{t("authorEditor.field.displayNameHelp")}</div>

                <label style={{ display: "flex", alignItems: "center", gap: 8, margin: "10px 0 4px" }}>
                  <input
                    type="checkbox"
                    checked={!!form.is_organization}
                    onChange={e => setForm({ ...form, is_organization: e.target.checked })}
                  />
                  <span style={{ fontSize: 12, color: "var(--text)" }}>
                    {t("authorEditor.field.isOrganization")}
                  </span>
                </label>
                <div style={hintStyle}>{t("authorEditor.field.isOrganizationHelp")}</div>

                {!form.is_organization && (
                  <>
                    <Row>
                      <Field label={t("authorEditor.field.givenName")}>
                        <input
                          value={form.given_name ?? ""}
                          onChange={e => setForm({ ...form, given_name: e.target.value })}
                          style={inputStyle}
                        />
                      </Field>
                      <Field label={t("authorEditor.field.middleName")}>
                        <input
                          value={form.middle_name ?? ""}
                          onChange={e => setForm({ ...form, middle_name: e.target.value })}
                          style={inputStyle}
                        />
                      </Field>
                      <Field label={t("authorEditor.field.familyName")}>
                        <input
                          value={form.family_name ?? ""}
                          onChange={e => setForm({ ...form, family_name: e.target.value })}
                          style={inputStyle}
                        />
                      </Field>
                    </Row>
                    <Row>
                      <Field label={t("authorEditor.field.suffix")}>
                        <input
                          value={form.suffix ?? ""}
                          onChange={e => setForm({ ...form, suffix: e.target.value })}
                          style={inputStyle}
                        />
                      </Field>
                      <Field label={t("authorEditor.field.nameParticle")}>
                        <input
                          value={form.name_particle ?? ""}
                          onChange={e => setForm({ ...form, name_particle: e.target.value })}
                          style={inputStyle}
                        />
                      </Field>
                    </Row>
                  </>
                )}
              </Section>

              {!form.is_organization && (
                <Section title={t("authorEditor.section.original")}>
                  <Row>
                    <Field label={t("authorEditor.field.nameOriginal")}>
                      <input
                        value={form.name_original ?? ""}
                        onChange={e => setForm({ ...form, name_original: e.target.value })}
                        style={inputStyle}
                        placeholder="関 茂樹"
                      />
                    </Field>
                    <Field label={t("authorEditor.field.originalScript")} narrow>
                      <select
                        value={form.original_script ?? ""}
                        onChange={e => setForm({ ...form, original_script: e.target.value || null })}
                        style={inputStyle}
                      >
                        <option value="">—</option>
                        {SCRIPT_OPTIONS.map(s => (
                          <option key={s} value={s}>{t(`authorEditor.script.${s}` as const)}</option>
                        ))}
                      </select>
                    </Field>
                  </Row>
                  <Row>
                    <Field label={t("authorEditor.field.familyNameOriginal")}>
                      <input
                        value={form.family_name_original ?? ""}
                        onChange={e => setForm({ ...form, family_name_original: e.target.value })}
                        style={inputStyle}
                      />
                    </Field>
                    <Field label={t("authorEditor.field.givenNameOriginal")}>
                      <input
                        value={form.given_name_original ?? ""}
                        onChange={e => setForm({ ...form, given_name_original: e.target.value })}
                        style={inputStyle}
                      />
                    </Field>
                  </Row>
                </Section>
              )}

              {!form.is_organization && (
                <Section title={t("authorEditor.section.reading")}>
                  <Row>
                    <Field label={t("authorEditor.field.readingFamily")}>
                      <input
                        value={form.reading_family ?? ""}
                        onChange={e => setForm({ ...form, reading_family: e.target.value })}
                        style={inputStyle}
                        placeholder="せき"
                      />
                    </Field>
                    <Field label={t("authorEditor.field.readingGiven")}>
                      <input
                        value={form.reading_given ?? ""}
                        onChange={e => setForm({ ...form, reading_given: e.target.value })}
                        style={inputStyle}
                        placeholder="もとき"
                      />
                    </Field>
                  </Row>
                </Section>
              )}

              <Section title={t("authorEditor.section.identifiers")}>
                <Row>
                  <Field label={t("authorEditor.field.orcid")}>
                    <div style={{ display: "flex", gap: 6 }}>
                      <input
                        value={form.orcid ?? ""}
                        onChange={e => {
                          setForm({ ...form, orcid: e.target.value });
                          setFetchSummary(null);
                        }}
                        style={{ ...inputStyle, flex: "1 1 auto" }}
                        placeholder="0000-0000-0000-0000"
                      />
                      <button
                        type="button"
                        onClick={handleFetchFromOrcid}
                        disabled={fetching || !ORCID_PATTERN.test((form.orcid ?? "").trim())}
                        title={t("authorEditor.fetch.button")}
                        style={{
                          ...secondaryButtonStyle,
                          padding: "5px 10px",
                          whiteSpace: "nowrap",
                          opacity:
                            fetching || !ORCID_PATTERN.test((form.orcid ?? "").trim())
                              ? 0.5
                              : 1,
                          cursor:
                            fetching || !ORCID_PATTERN.test((form.orcid ?? "").trim())
                              ? "default"
                              : "pointer",
                        }}
                      >
                        {fetching
                          ? t("authorEditor.fetch.fetching")
                          : t("authorEditor.fetch.button")}
                      </button>
                    </div>
                  </Field>
                </Row>
                {fetchSummary && (
                  <div style={{
                    marginTop: 4, fontSize: 11, color: "var(--text-mute)",
                    background: "var(--surface-2)", padding: "5px 8px", borderRadius: 4,
                  }}>
                    {fetchSummary}
                  </div>
                )}
                <IdentifiersEditor
                  rows={form.identifiers ?? []}
                  onChange={rows => setForm({ ...form, identifiers: rows })}
                />
              </Section>

              <Section title={t("authorEditor.section.extra")}>
                <Row>
                  <Field label={t("authorEditor.field.email")}>
                    <input
                      value={form.email ?? ""}
                      onChange={e => setForm({ ...form, email: e.target.value })}
                      style={inputStyle}
                      placeholder="user@example.com"
                    />
                  </Field>
                  <Field label={t("authorEditor.field.homepageUrl")}>
                    <input
                      value={form.homepage_url ?? ""}
                      onChange={e => setForm({ ...form, homepage_url: e.target.value })}
                      style={inputStyle}
                      placeholder="https://"
                    />
                  </Field>
                </Row>
                <Row>
                  <Field label={t("authorEditor.field.notes")}>
                    <textarea
                      value={form.notes ?? ""}
                      onChange={e => setForm({ ...form, notes: e.target.value })}
                      rows={3}
                      style={{ ...inputStyle, fontFamily: "inherit", resize: "vertical" }}
                    />
                  </Field>
                </Row>
                <div style={hintStyle}>{t("authorEditor.field.notesHelp")}</div>
              </Section>

              <Section title={t("authorEditor.section.merge")}>
                <div style={hintStyle}>{t("authorEditor.merge.intro")}</div>
                {confirmMergeFrom ? (
                  <div style={{
                    marginTop: 10,
                    padding: "10px 12px",
                    border: "1px solid var(--border-strong)",
                    background: "var(--surface-2)",
                    borderRadius: 6,
                    fontSize: 12, color: "var(--text)",
                  }}>
                    <div style={{ marginBottom: 8 }}>
                      {t("authorEditor.merge.confirmBody", {
                        from: confirmMergeFrom.name,
                        into: form.name,
                      })}
                    </div>
                    <div style={{ display: "flex", gap: 8 }}>
                      <button
                        onClick={handleMerge}
                        disabled={merging}
                        style={{ ...primaryButtonStyle, background: "var(--danger-strong, #c0392b)" }}
                      >
                        {merging ? t("authorEditor.merge.submitting") : t("authorEditor.merge.submit")}
                      </button>
                      <button
                        onClick={() => setConfirmMergeFrom(null)}
                        disabled={merging}
                        style={secondaryButtonStyle}
                      >
                        {t("authorEditor.cancel")}
                      </button>
                    </div>
                  </div>
                ) : (
                  <>
                    <input
                      value={mergeQuery}
                      onChange={e => setMergeQuery(e.target.value)}
                      placeholder={t("authorEditor.merge.searchPlaceholder")}
                      style={{ ...inputStyle, marginTop: 8 }}
                    />
                    {mergeQuery.trim().length >= 2 && (
                      <div style={{
                        marginTop: 6,
                        border: "1px solid var(--border)",
                        borderRadius: 6,
                        maxHeight: 160, overflowY: "auto",
                      }}>
                        {mergeCandidates.length === 0 ? (
                          <div style={{ padding: 10, fontSize: 12, color: "var(--text-faint)" }}>
                            {t("authorEditor.merge.noResults")}
                          </div>
                        ) : (
                          mergeCandidates.map(c => (
                            <button
                              key={c.id}
                              onClick={() => setConfirmMergeFrom(c)}
                              style={{
                                display: "block", width: "100%", textAlign: "left",
                                padding: "8px 10px",
                                border: "none", background: "transparent",
                                fontSize: 12, color: "var(--text)", cursor: "pointer",
                                borderBottom: "1px solid var(--border)",
                              }}
                            >
                              <div style={{ fontWeight: 500 }}>{c.name}</div>
                              {(c.orcid || c.name_original) && (
                                <div style={{ fontSize: 10, color: "var(--text-faint)", marginTop: 2 }}>
                                  {c.name_original ?? ""} {c.orcid ? `· ORCID ${c.orcid}` : ""}
                                </div>
                              )}
                            </button>
                          ))
                        )}
                      </div>
                    )}
                  </>
                )}
              </Section>
            </>
          )}

          {validationError && (
            <div style={{ marginTop: 12, fontSize: 12, color: "var(--danger-strong, #c0392b)" }}>
              {validationError}
            </div>
          )}
          {error && (
            <div style={{ marginTop: 12, fontSize: 12, color: "var(--danger-strong, #c0392b)" }}>
              {error}
            </div>
          )}
        </div>

        <footer style={{
          padding: "10px 22px 14px",
          borderTop: "1px solid var(--border)",
          display: "flex", justifyContent: "flex-end", gap: 8,
        }}>
          <button onClick={onClose} disabled={saving || merging} style={secondaryButtonStyle}>
            {t("authorEditor.cancel")}
          </button>
          <button onClick={handleSave} disabled={!canSave} style={primaryButtonStyle}>
            {saving ? t("authorEditor.saving") : t("authorEditor.save")}
          </button>
        </footer>
      </div>
    </div>
  );
}

// ── 内側コンポーネント ────────────────────────────────────────────────────────

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section style={{ marginBottom: 18 }}>
      <div style={{
        fontSize: 10.5, fontWeight: 600, color: "var(--text-faint)",
        textTransform: "uppercase", letterSpacing: "0.06em", marginBottom: 8,
      }}>{title}</div>
      {children}
    </section>
  );
}

function Row({ children }: { children: React.ReactNode }) {
  return (
    <div style={{ display: "flex", gap: 8, marginBottom: 6 }}>{children}</div>
  );
}

function Field({
  label,
  required,
  narrow,
  children,
}: {
  label: string;
  required?: boolean;
  narrow?: boolean;
  children: React.ReactNode;
}) {
  return (
    <label style={{
      flex: narrow ? "0 0 160px" : "1 1 auto",
      display: "flex", flexDirection: "column", gap: 3,
    }}>
      <span style={{ fontSize: 11, color: "var(--text-mute)" }}>
        {label}{required && <span style={{ color: "var(--danger-strong, #c0392b)" }}>*</span>}
      </span>
      {children}
    </label>
  );
}

function IdentifiersEditor({
  rows,
  onChange,
}: {
  rows: AuthorIdentifierInput[];
  onChange: (next: AuthorIdentifierInput[]) => void;
}) {
  const { t } = useTranslation();
  const setRow = (i: number, patch: Partial<AuthorIdentifierInput>) =>
    onChange(rows.map((r, idx) => (idx === i ? { ...r, ...patch } : r)));
  const removeRow = (i: number) => onChange(rows.filter((_, idx) => idx !== i));
  const addRow = () =>
    onChange([...rows, { scheme: "dblp", value: "", url: null }]);

  return (
    <div style={{ marginTop: 8, display: "flex", flexDirection: "column", gap: 6 }}>
      {rows.map((r, i) => (
        <div key={i} style={{ display: "flex", gap: 6, alignItems: "center" }}>
          <select
            value={r.scheme}
            onChange={e => setRow(i, { scheme: e.target.value })}
            style={{ ...inputStyle, flex: "0 0 160px" }}
          >
            {SCHEME_OPTIONS.filter(s => s !== "orcid").map(s => (
              <option key={s} value={s}>{t(`authorEditor.scheme.${s}` as const)}</option>
            ))}
          </select>
          <input
            value={r.value}
            onChange={e => setRow(i, { value: e.target.value })}
            placeholder={t("authorEditor.identifier.valuePlaceholder")}
            style={{ ...inputStyle, flex: "1 1 auto" }}
          />
          <input
            value={r.url ?? ""}
            onChange={e => setRow(i, { url: e.target.value })}
            placeholder={t("authorEditor.identifier.urlPlaceholder")}
            style={{ ...inputStyle, flex: "1 1 auto" }}
          />
          <button
            type="button"
            onClick={() => removeRow(i)}
            style={{
              ...secondaryButtonStyle,
              padding: "5px 8px", fontSize: 11,
            }}
          >
            ×
          </button>
        </div>
      ))}
      <button
        type="button"
        onClick={addRow}
        style={{
          ...secondaryButtonStyle,
          alignSelf: "flex-start", fontSize: 11, padding: "4px 10px",
        }}
      >
        + {t("authorEditor.identifier.addRow")}
      </button>
    </div>
  );
}

// ── バリデーション ────────────────────────────────────────────────────────────

/**
 * ORCID から取得した AuthorInput を現在のフォームに**非破壊的に**マージする。
 *
 * 設計方針:
 * - 個別スカラーフィールド: 既存が空/未入力のときだけ fetched の値で埋める。
 *   ユーザーが既に手入力していたものは温存（混乱を避ける）。
 * - identifiers: 既存と scheme で diff を取り、未登録の scheme だけを追加。
 *   ORCID の scheme は別フィールドに専用入力があるので identifiers 側からは除外。
 * - is_organization: ORCID は個人専用なので触らない。
 */
function mergeFetchedIntoForm(
  current: AuthorInput,
  fetched: AuthorInput,
): { next: AuthorInput; fieldsFilled: number; identifiersAdded: number } {
  let fieldsFilled = 0;
  const fillScalar = (cur: string | null | undefined, nxt: string | null | undefined) => {
    const c = (cur ?? "").trim();
    const n = (nxt ?? "").trim();
    if (c === "" && n !== "") {
      fieldsFilled += 1;
      return n;
    }
    return cur ?? null;
  };

  const next: AuthorInput = {
    ...current,
    name: current.name.trim() === "" && fetched.name.trim() !== "" ? (fieldsFilled++, fetched.name) : current.name,
    given_name: fillScalar(current.given_name, fetched.given_name),
    middle_name: fillScalar(current.middle_name, fetched.middle_name),
    family_name: fillScalar(current.family_name, fetched.family_name),
    name_original: fillScalar(current.name_original, fetched.name_original),
    original_script: fillScalar(current.original_script, fetched.original_script),
    email: fillScalar(current.email, fetched.email),
    homepage_url: fillScalar(current.homepage_url, fetched.homepage_url),
  };

  // identifiers: scheme で重複排除（既存優先 + orcid は専用フィールドにあるので捨てる）
  const existingSchemes = new Set((current.identifiers ?? []).map(i => i.scheme));
  const incoming = (fetched.identifiers ?? []).filter(
    i => i.scheme !== "orcid" && !existingSchemes.has(i.scheme) && i.scheme.trim() !== "" && i.value.trim() !== "",
  );
  if (incoming.length > 0) {
    next.identifiers = [...(current.identifiers ?? []), ...incoming];
  }

  return { next, fieldsFilled, identifiersAdded: incoming.length };
}

/**
 * バリデーション失敗時は i18n キーを返し、成功時は null。
 * 戻り値は具体的なキー union にしておくと、呼び出し側で `t(key)` がオーバーロード解決できる。
 */
type ValidationKey =
  | "authorEditor.validation.nameRequired"
  | "authorEditor.validation.orcidFormat"
  | "authorEditor.validation.urlFormat"
  | "authorEditor.validation.identifierIncomplete";

function validate(form: AuthorInput | null): ValidationKey | null {
  if (!form) return null;
  if (form.name.trim() === "") return "authorEditor.validation.nameRequired";
  if (form.orcid && form.orcid.trim() !== "" && !ORCID_PATTERN.test(form.orcid.trim())) {
    return "authorEditor.validation.orcidFormat";
  }
  if (form.homepage_url && form.homepage_url.trim() !== "" && !URL_PATTERN.test(form.homepage_url.trim())) {
    return "authorEditor.validation.urlFormat";
  }
  for (const i of form.identifiers ?? []) {
    if ((i.scheme.trim() === "") !== (i.value.trim() === "")) {
      return "authorEditor.validation.identifierIncomplete";
    }
    if (i.url && i.url.trim() !== "" && !URL_PATTERN.test(i.url.trim())) {
      return "authorEditor.validation.urlFormat";
    }
    if (i.scheme.trim() === "orcid" && i.value.trim() !== "" && !ORCID_PATTERN.test(i.value.trim())) {
      return "authorEditor.validation.orcidFormat";
    }
  }
  return null;
}

// ── 共通スタイル ──────────────────────────────────────────────────────────────

const inputStyle: React.CSSProperties = {
  width: "100%",
  padding: "5px 8px",
  borderRadius: 5,
  border: "1px solid var(--border)",
  background: "var(--surface)",
  color: "var(--text)",
  fontSize: 12,
};

const hintStyle: React.CSSProperties = {
  fontSize: 11, color: "var(--text-faint)", lineHeight: 1.5, marginTop: 2,
};

const primaryButtonStyle: React.CSSProperties = {
  padding: "6px 14px", borderRadius: 5,
  border: "1px solid var(--border-strong)",
  background: "var(--accent-strong)", color: "white",
  fontSize: 12, fontWeight: 500, cursor: "pointer",
};

const secondaryButtonStyle: React.CSSProperties = {
  padding: "6px 12px", borderRadius: 5,
  border: "1px solid var(--border-strong)",
  background: "var(--surface)", color: "var(--text)",
  fontSize: 12, cursor: "pointer",
};
