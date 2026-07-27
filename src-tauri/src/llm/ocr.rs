//! LLM Vision で画像 1 枚を扱う経路（プロバイダ非依存）。
//! - [`ocr_image`]: ページ画像 → 文字起こしテキスト（スキャン PDF の全文索引用）。
//! - [`describe_image`]: 図の crop 画像 → 代替テキスト（LCIR Phase 8c・`node_alt_texts`）。
//!
//! どちらも同じ配管（`ContentBlock::Image` + `stream_chat`）で、**system プロンプトだけが違う**。

use crate::llm::{provider_for, ChatMessage, ContentBlock, LlmError, Role};

pub const OCR_SYSTEM_PROMPT: &str = "You are an OCR engine. Transcribe ALL text visible in the \
image faithfully, preserving reading order and paragraph structure. Do not summarize, translate, \
or add any commentary — output only the transcribed text. If the page has no text, output nothing.";

/// 図の代替テキスト生成（Phase 8c）の system プロンプト。**見えるものだけを書かせる**方針
/// （AI 推定を原資料の言い換えに見せない・論文の主張を推測させない）。応答が空文字列なら
/// 呼び出し側は「生成できなかった」として行を作らない（誤った説明より欠損を選ぶ）。
pub const ALT_TEXT_SYSTEM_PROMPT: &str = "You write alt text for figures from academic papers, \
for readers who cannot see the image. Given one cropped figure, reply with 1-3 factual sentences: \
first the figure type (plot, diagram, graph, schematic, photograph, ...), then the concrete visible \
content — axis labels with units, legend and series names, node and edge labels, visible trends, \
panel layout (e.g. \"three panels labelled (a) to (c)\"). Transcribe short labels verbatim when \
legible. Describe ONLY what is visible: do not infer the paper's conclusions, do not guess numbers \
or words you cannot read, and do not invent any detail. Do not open with \"This image shows\", and \
do not add commentary, markdown, or headings. If the crop is blank or its content is indiscernible, \
output nothing.";

/// 画像 1 枚（base64）を OCR してテキストを返す。`media_type` 例: "image/png"。
pub async fn ocr_image(
    provider: &str,
    model: &str,
    api_key: &str,
    media_type: &str,
    data_base64: &str,
) -> Result<String, LlmError> {
    vision_call(
        provider,
        model,
        api_key,
        OCR_SYSTEM_PROMPT,
        "Transcribe the text in this page image.",
        media_type,
        data_base64,
    )
    .await
}

/// 図の crop 画像 1 枚（base64）を説明して代替テキストを返す（Phase 8c）。
/// 呼び出し元は結果を `origin='llm_inference'` + `confidence` + `model` 付きで保存する。
pub async fn describe_image(
    provider: &str,
    model: &str,
    api_key: &str,
    media_type: &str,
    data_base64: &str,
) -> Result<String, LlmError> {
    vision_call(
        provider,
        model,
        api_key,
        ALT_TEXT_SYSTEM_PROMPT,
        "Describe this figure crop from a research paper.",
        media_type,
        data_base64,
    )
    .await
}

/// 画像 1 枚 + テキスト 1 行のユーザーメッセージを投げて応答テキストを返す共通経路
/// （tool は渡さず、ストリームは捨てて最終テキストだけを使う）。
async fn vision_call(
    provider: &str,
    model: &str,
    api_key: &str,
    system_prompt: &str,
    user_text: &str,
    media_type: &str,
    data_base64: &str,
) -> Result<String, LlmError> {
    if api_key.trim().is_empty() {
        return Err(LlmError::MissingApiKey);
    }
    let p = provider_for(provider)?;
    let messages = vec![ChatMessage {
        role: Role::User,
        content: vec![
            ContentBlock::text(user_text),
            ContentBlock::Image {
                media_type: media_type.to_string(),
                data: data_base64.to_string(),
            },
        ],
        tool_calls: None,
        tool_call_id: None,
    }];
    let mut noop = |_: &str| {};
    let result = p
        .stream_chat(api_key, model, system_prompt, &messages, &[], &mut noop)
        .await?;
    Ok(result.text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// API キー未設定は**ネットワークに出る前に**弾く（バッチが空キーで全図分叩かないため）。
    #[tokio::test]
    async fn missing_api_key_is_rejected_before_any_request() {
        for r in [
            ocr_image("openai", "gpt-4o-mini", "  ", "image/png", "AAAA").await,
            describe_image("openai", "gpt-4o-mini", "", "image/png", "AAAA").await,
        ] {
            assert!(matches!(r, Err(LlmError::MissingApiKey)));
        }
    }

    /// Phase 8c: **実ライブラリの一括生成をヘッドレスで回す**手動ハーネス。アプリのボタン
    /// （Tauri コマンド `generate_vision_alt_texts`）と同じ対象述語・同じプロンプト・同じアクセサを
    /// 使うが Tauri ランタイム無しで動く（ターミナルから長時間バッチを流したいとき用）。
    /// **1 図ごとに課金される。** 冪等（行がある図は対象外）なので中断しても再実行で続きから進む。
    ///
    /// 本番との差: ①版の陳腐化チェックは省略（実行中に再構築しない前提）②連続失敗の打ち切りを
    /// 10 件に固定（長時間の無人実行で系統的失敗を焼き続けないため）。
    ///
    /// ```text
    /// LCIR_ALT_TEXT_DB=<live.db> LCIR_ALT_TEXT_APPDIR=<app data dir> \
    ///   LCIR_ALT_TEXT_KEY=<api key> [LCIR_ALT_TEXT_LIMIT=N] \
    ///   cargo test --lib generate_alt_texts_for_library -- --ignored --nocapture
    /// ```
    #[tokio::test]
    #[ignore = "manual billed batch; needs LCIR_ALT_TEXT_* env"]
    async fn generate_alt_texts_for_library() {
        use base64::Engine;
        use std::collections::HashMap;
        use std::time::Duration;
        let (Ok(db), Ok(appdir), Ok(key)) = (
            std::env::var("LCIR_ALT_TEXT_DB"),
            std::env::var("LCIR_ALT_TEXT_APPDIR"),
            std::env::var("LCIR_ALT_TEXT_KEY"),
        ) else {
            eprintln!("skip: set LCIR_ALT_TEXT_DB / LCIR_ALT_TEXT_APPDIR / LCIR_ALT_TEXT_KEY");
            return;
        };
        let limit: usize = std::env::var("LCIR_ALT_TEXT_LIMIT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(usize::MAX);

        // 稼働中アプリと同居するので busy_timeout を長めに取る（WAL で読み書きは併存できる）。
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&db)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(30));
        let pool = sqlx::SqlitePool::connect_with(opts).await.expect("open db");

        let (provider, model) = crate::llm::tools::ocr::resolve_ocr_provider(&pool)
            .await
            .expect("resolve provider/model");
        let targets = crate::db::node_alt_texts::figures_missing_alt_text(
            &pool,
            crate::db::node_alt_texts::AltTextTargetFilter::default(),
        )
        .await
        .expect("targets");
        let total = targets.len().min(limit);
        eprintln!("[8c-batch] provider={provider} model={model} targets={total}");

        let (mut generated, mut skipped, mut failed) = (0usize, 0usize, 0usize);
        let mut consecutive_failures = 0u32;
        let mut api_calls = 0u32;
        let mut described_by_sha: HashMap<String, String> = HashMap::new();

        for (i, t) in targets.into_iter().take(limit).enumerate() {
            let abs = std::path::Path::new(&appdir).join(&t.relative_path);
            // 同一ラン内でバイト同一 crop を二度課金しない（本番と同じ指紋メモ）。
            let cached = described_by_sha.get(&t.asset_sha256).cloned();
            let text = match cached {
                Some(text) => Some(text),
                None => {
                    let Ok(bytes) = std::fs::read(&abs) else {
                        skipped += 1;
                        eprintln!("[{}/{total}] skip (crop missing) node={}", i + 1, t.node_id);
                        continue;
                    };
                    if api_calls > 0 {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                    api_calls += 1;
                    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                    match describe_image(&provider, &model, &key, &t.mime_type, &b64).await {
                        Ok(text) if text.trim().is_empty() => {
                            skipped += 1;
                            consecutive_failures = 0;
                            eprintln!("[{}/{total}] skip (empty) node={}", i + 1, t.node_id);
                            continue;
                        }
                        Ok(text) => {
                            let text: String = text.trim().chars().take(4000).collect();
                            described_by_sha.insert(t.asset_sha256.clone(), text.clone());
                            Some(text)
                        }
                        Err(e) => {
                            failed += 1;
                            consecutive_failures += 1;
                            eprintln!("[{}/{total}] FAIL node={} {e}", i + 1, t.node_id);
                            if consecutive_failures >= 10 {
                                eprintln!("[8c-batch] aborting after 10 consecutive failures");
                                break;
                            }
                            continue;
                        }
                    }
                }
            };
            let text = text.expect("text");
            match crate::db::node_alt_texts::insert_alt_text(
                &pool,
                &crate::db::node_alt_texts::NewAltText {
                    node_id: t.node_id,
                    document_version_id: t.document_version_id,
                    source_asset_sha256: &t.asset_sha256,
                    text: &text,
                    origin: crate::document_ir::Origin::LlmInference.as_str(),
                    confidence: Some(0.5),
                    model: Some(&model),
                    carried_from_version_id: None,
                },
            )
            .await
            {
                Ok(_) => {
                    generated += 1;
                    consecutive_failures = 0;
                    eprintln!(
                        "[{}/{total}] ok node={} {}",
                        i + 1,
                        t.node_id,
                        text.chars().take(70).collect::<String>()
                    );
                }
                Err(e) => {
                    failed += 1;
                    consecutive_failures += 1;
                    eprintln!("[{}/{total}] SAVE FAIL node={} {e}", i + 1, t.node_id);
                }
            }
        }
        eprintln!(
            "[8c-batch] done: generated={generated} skipped={skipped} failed={failed} (api_calls={api_calls})"
        );
    }

    /// Phase 8c の手動スモーク: 実際の crop PNG 1 枚で alt text 生成プロンプトを確かめる。
    /// **API キーは env で渡す**（テストバイナリから keychain を触らない）・**1 枚だけ課金される**。
    ///
    /// ```text
    /// LCIR_SMOKE_IMAGE=<crop.png> LCIR_SMOKE_VISION_KEY=<api key> \
    ///   [LCIR_SMOKE_VISION_PROVIDER=openai] [LCIR_SMOKE_VISION_MODEL=gpt-4o-mini] \
    ///   cargo test --lib vision_alt_text_real_image -- --ignored --nocapture
    /// ```
    /// crop PNG は `lcir_build_real_pdf` を `LCIR_SMOKE_KEEP=1` で回すと一時 appdir に残る。
    #[tokio::test]
    #[ignore = "manual vision smoke test; needs LCIR_SMOKE_IMAGE + LCIR_SMOKE_VISION_KEY (billed)"]
    async fn vision_alt_text_real_image() {
        use base64::Engine;
        let (Ok(path), Ok(key)) = (
            std::env::var("LCIR_SMOKE_IMAGE"),
            std::env::var("LCIR_SMOKE_VISION_KEY"),
        ) else {
            eprintln!("skip: set LCIR_SMOKE_IMAGE / LCIR_SMOKE_VISION_KEY");
            return;
        };
        let provider =
            std::env::var("LCIR_SMOKE_VISION_PROVIDER").unwrap_or_else(|_| "openai".to_string());
        let model =
            std::env::var("LCIR_SMOKE_VISION_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());
        let bytes = std::fs::read(&path).expect("read LCIR_SMOKE_IMAGE");
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        let text = describe_image(&provider, &model, &key, "image/png", &b64)
            .await
            .expect("vision call");
        eprintln!("[phase8c] {provider}/{model} on {path}\n  alt_text = {text:?}");
        assert!(!text.trim().is_empty(), "空応答なら行は作らない設計だが、通常は説明が返る");
    }
}
