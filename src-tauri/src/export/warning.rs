//! エクスポート時の欠落警告（Phase 9a・debt-8）。
//!
//! ロードマップ Phase 9 の完了条件「**LCIR 固有情報が失われる場合に警告を出せる**」を担う。
//! Phase 9b（HTML+MathML / JATS / TEI）も同じチャネルを共有する前提で、形式ごとの表現力を
//! [`FormatCapabilities`] で宣言し、警告はそこから機械的に導く。
//!
//! `document_ir::validation` とは役割が違う: validation は「**不正な LCIR を弾く**」（エラー・
//! 書き出しを中止する）。こちらは「**正しい LCIR だが、この出力形式では運べない**」（警告・
//! 書き出しは成功している）。混ぜないこと。
//!
//! **狼少年にしないための 3 規約**:
//!
//! 1. **その文書に実際に存在するデータだけ報告する。** relations が 0 本の文書で
//!    「関係が失われる」とは言わない。どの文書でも必ず真になる一般論は警告にしない。
//! 2. **どの形式でも常に落ちる縮約は警告にしない。** ノード id・`content_key`・schema URI 等は
//!    Markdown に載らないのが当たり前で、報告しても行動につながらない（LCIR JSON で取れる）。
//! 3. **1 つの損失を 1 コードで報告する。** 同じ事実を粒度違いで複数回出さない。
//!
//! レンダラ（`export::markdown`）には**一切触らない**。「何バイト出力されたか」を覗く方式は
//! 「ノードが出力に触れたか」しか見えず、`render_node` の分岐と二重管理になるため採らない。

use serde::Serialize;

use crate::document_ir::LcirDocument;

/// 警告の重さ。`warn` = 意味のある情報が落ちる / `info` = 落ちるが派生ビューの性質上想定内。
/// 3 段目（部分変換）が要るのは 9b-2（JATS）なので、必要になってから足す。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportSeverity {
    Warn,
    Info,
}

impl ExportSeverity {
    pub fn as_str(self) -> &'static str {
        match self {
            ExportSeverity::Warn => "warn",
            ExportSeverity::Info => "info",
        }
    }
}

/// 警告の機械可読な安定 ID。フロントの i18n キー（`detailPanel.lcirExportWarn.<code>`）と
/// CLI の stderr 行に使うので、**一度出したら綴りを変えない**。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportWarningCode {
    /// 参照グラフ（Phase 6a・`node_relations`）が出力に現れない。
    RelationsDropped,
    /// 記号定義（Phase 6b・`symbols`/`symbol_occurrences`）が出力に現れない。
    SymbolsDropped,
    /// 推定由来（`layout_model` 等）であることの表明が出力に現れない。
    InferredProvenanceDropped,
    /// PDF 座標（`source_fragments` の bbox）が出力に現れない。
    SourceFragmentsDropped,
    /// アセット（図の crop PNG）の実体を同梱していない（参照だけ／それも出さない）。
    AssetsNotEmbedded,
    /// 表の縦結合セル（`rowspan`）が平坦化される。
    TableRowspanFlattened,
}

impl ExportWarningCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ExportWarningCode::RelationsDropped => "relations_dropped",
            ExportWarningCode::SymbolsDropped => "symbols_dropped",
            ExportWarningCode::InferredProvenanceDropped => "inferred_provenance_dropped",
            ExportWarningCode::SourceFragmentsDropped => "source_fragments_dropped",
            ExportWarningCode::AssetsNotEmbedded => "assets_not_embedded",
            ExportWarningCode::TableRowspanFlattened => "table_rowspan_flattened",
        }
    }

    pub fn severity(self) -> ExportSeverity {
        match self {
            ExportWarningCode::RelationsDropped
            | ExportWarningCode::SymbolsDropped
            | ExportWarningCode::InferredProvenanceDropped => ExportSeverity::Warn,
            ExportWarningCode::SourceFragmentsDropped
            | ExportWarningCode::AssetsNotEmbedded
            | ExportWarningCode::TableRowspanFlattened => ExportSeverity::Info,
        }
    }

    /// 全 variant（テストと docs 生成用）。
    pub fn all() -> &'static [ExportWarningCode] {
        &[
            ExportWarningCode::RelationsDropped,
            ExportWarningCode::SymbolsDropped,
            ExportWarningCode::InferredProvenanceDropped,
            ExportWarningCode::SourceFragmentsDropped,
            ExportWarningCode::AssetsNotEmbedded,
            ExportWarningCode::TableRowspanFlattened,
        ]
    }
}

/// 1 件の警告。`count` は**落ちた対象の実数**（辺数・記号数・ノード数…）で、
/// `detail` は内訳の短い人間向け補足（種別ごとの件数など。翻訳はしない）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExportWarning {
    pub code: ExportWarningCode,
    pub severity: ExportSeverity,
    pub count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// 出力形式が「何を運べるか」の宣言。警告はここから機械的に導くので、
/// 9b で形式を足すときは const を 1 つ増やすだけで済む。
#[derive(Debug, Clone, Copy)]
pub struct FormatCapabilities {
    /// ノード間の関係（参照グラフ）を表現できるか。
    pub relations: bool,
    /// 記号定義を表現できるか。
    pub symbols: bool,
    /// ノードごとの推定/原文由来の別（origin・confidence）を表現できるか。
    pub node_provenance: bool,
    /// PDF 座標（bbox）を表現できるか。
    pub coordinates: bool,
    /// アセットの実体を同梱できるか（参照だけでは false）。
    pub embedded_assets: bool,
    /// 表の結合セル（rowspan）を表現できるか。
    pub cell_spans: bool,
}

/// 構造付き Markdown（`export::markdown`）。人間・LLM 向けの派生ビューで、
/// 関係・記号・座標・provenance はいずれも運べない。図は存在マーカーだけで実体は同梱しない。
pub const MARKDOWN: FormatCapabilities = FormatCapabilities {
    relations: false,
    symbols: false,
    node_provenance: false,
    coordinates: false,
    embedded_assets: false,
    cell_spans: false,
};

/// LCIR JSON。派生ビューとしては無損失で、唯一の欠落はアセット**実体**の非同梱
/// （`relative_path` + sha256 のメタデータ参照だけを載せる）。
pub const LCIR_JSON: FormatCapabilities = FormatCapabilities {
    relations: true,
    symbols: true,
    node_provenance: true,
    coordinates: true,
    embedded_assets: false,
    cell_spans: true,
};

/// 「推定」を表す origin（roadmap §16・これらが落ちると AI 推定と原文由来の区別がつかなくなる）。
const INFERRED_ORIGINS: &[&str] = &["layout_model", "llm_inference", "math_recognition", "ocr"];

/// 警告の収集先。`count` が 0 のコードは最終的に落とす（存在しない損失を報告しない）。
#[derive(Debug, Default)]
pub struct WarningSink {
    hits: Vec<(ExportWarningCode, i64, Option<String>)>,
}

impl WarningSink {
    pub fn new() -> Self {
        Self::default()
    }

    /// 警告を 1 件積む。`count == 0` は積まない（規約 1）。
    fn push(&mut self, code: ExportWarningCode, count: i64, detail: Option<String>) {
        if count > 0 {
            self.hits.push((code, count, detail));
        }
    }

    /// `(severity, code)` の決定的な順で確定する（同じ文書からは常に同じ並びが出る）。
    pub fn finish(mut self) -> Vec<ExportWarning> {
        self.hits.sort_by_key(|(code, _, _)| (code.severity(), *code));
        self.hits
            .into_iter()
            .map(|(code, count, detail)| ExportWarning {
                code,
                severity: code.severity(),
                count,
                detail,
            })
            .collect()
    }
}

/// 文書と出力形式から欠落警告を収集する。**文書を 1 パス走査するだけの純関数**。
pub fn collect_document_warnings(
    doc: &LcirDocument,
    caps: &FormatCapabilities,
    sink: &mut WarningSink,
) {
    if !caps.relations {
        let mut by_type: std::collections::BTreeMap<&str, i64> = Default::default();
        for r in &doc.relations {
            *by_type.entry(r.relation_type.as_str()).or_insert(0) += 1;
        }
        sink.push(
            ExportWarningCode::RelationsDropped,
            doc.relations.len() as i64,
            breakdown(&by_type),
        );
    }
    if !caps.symbols {
        sink.push(ExportWarningCode::SymbolsDropped, doc.symbols.len() as i64, None);
    }

    // ノード走査（provenance / 座標 / アセット / rowspan を 1 周で数える）。
    let mut inferred: std::collections::BTreeMap<&str, i64> = Default::default();
    let mut fragments = 0i64;
    let mut assets = 0i64;
    let mut rowspan_cells = 0i64;
    for n in &doc.nodes {
        if let Some(o) = n.origin.as_deref() {
            if INFERRED_ORIGINS.contains(&o) {
                *inferred.entry(o).or_insert(0) += 1;
            }
        }
        fragments += n.source_fragments.len() as i64;
        assets += n.assets.len() as i64;
        if n.kind == "table" {
            rowspan_cells += count_rowspan_cells(n);
        }
    }

    if !caps.node_provenance {
        let total: i64 = inferred.values().sum();
        sink.push(
            ExportWarningCode::InferredProvenanceDropped,
            total,
            breakdown(&inferred),
        );
    }
    if !caps.coordinates {
        sink.push(ExportWarningCode::SourceFragmentsDropped, fragments, None);
    }
    if !caps.embedded_assets {
        sink.push(ExportWarningCode::AssetsNotEmbedded, assets, None);
    }
    if !caps.cell_spans {
        sink.push(ExportWarningCode::TableRowspanFlattened, rowspan_cells, None);
    }
}

/// table ノードの payload から `rowspan > 1` のセル数を数える（形が違う payload は 0）。
fn count_rowspan_cells(n: &crate::document_ir::LcirNode) -> i64 {
    let Some(rows) = n.payload.as_ref().and_then(|p| p.get("rows")?.as_array()) else {
        return 0;
    };
    rows.iter()
        .filter_map(|r| r.get("cells")?.as_array())
        .flatten()
        .filter(|c| c.get("rowspan").and_then(|v| v.as_i64()).unwrap_or(1) > 1)
        .count() as i64
}

/// `{"a": 2, "b": 1}` → `"a: 2, b: 1"`。空なら None。
fn breakdown(map: &std::collections::BTreeMap<&str, i64>) -> Option<String> {
    if map.is_empty() {
        return None;
    }
    Some(
        map.iter()
            .map(|(k, v)| format!("{k}: {v}"))
            .collect::<Vec<_>>()
            .join(", "),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document_ir::{
        BBox, LcirAsset, LcirFragment, LcirNode, LcirRelation, LcirSource, LcirSymbol,
    };

    fn node(id: i64, kind: &str, origin: Option<&str>) -> LcirNode {
        LcirNode {
            id,
            kind: kind.to_string(),
            ordinal: 0,
            parent_id: None,
            plain_text: None,
            origin: origin.map(|s| s.to_string()),
            confidence: None,
            payload: None,
            math: None,
            source_fragments: Vec::new(),
            assets: Vec::new(),
            alt_text: None,
        }
    }

    fn doc(nodes: Vec<LcirNode>) -> LcirDocument {
        LcirDocument {
            schema: crate::document_ir::schema::SCHEMA_URI.to_string(),
            schema_version: crate::document_ir::schema::SCHEMA_VERSION.to_string(),
            version_id: 1,
            content_key: "k".to_string(),
            source: LcirSource {
                sha256: "s".to_string(),
                mime_type: "application/pdf".to_string(),
                extractor_name: "lumencite-pdfium".to_string(),
                extractor_version: "0.7.0".to_string(),
            },
            coordinate_space: None,
            nodes,
            relations: Vec::new(),
            symbols: Vec::new(),
        }
    }

    fn symbol() -> LcirSymbol {
        LcirSymbol {
            id: 1,
            surface_form: "U".to_string(),
            normalized_form: None,
            description: None,
            symbol_type: None,
            defined_at_node_id: None,
            scope_node_id: None,
            confidence: None,
            origin: None,
            occurrences: Vec::new(),
        }
    }

    fn codes(ws: &[ExportWarning]) -> Vec<&'static str> {
        ws.iter().map(|w| w.code.as_str()).collect()
    }

    fn collect(d: &LcirDocument, caps: &FormatCapabilities) -> Vec<ExportWarning> {
        let mut sink = WarningSink::new();
        collect_document_warnings(d, caps, &mut sink);
        sink.finish()
    }

    #[test]
    fn warning_codes_are_unique_snake_case_and_have_severity() {
        let mut seen = std::collections::HashSet::new();
        for c in ExportWarningCode::all() {
            let s = c.as_str();
            assert!(seen.insert(s), "code が重複: {s}");
            assert!(
                s.chars().all(|ch| ch.is_ascii_lowercase() || ch == '_'),
                "snake_case であること: {s}"
            );
            // severity() が網羅的（panic しない）ことの確認も兼ねる。
            assert!(matches!(
                c.severity(),
                ExportSeverity::Warn | ExportSeverity::Info
            ));
        }
    }

    /// 狼少年にしない核。データが無ければ警告は 1 件も出ない。
    #[test]
    fn empty_document_produces_no_warnings() {
        let d = doc(vec![node(1, "document", None)]);
        assert!(collect(&d, &MARKDOWN).is_empty(), "{:?}", collect(&d, &MARKDOWN));
        assert!(collect(&d, &LCIR_JSON).is_empty());
    }

    #[test]
    fn tex_document_does_not_warn_about_inferred_provenance() {
        // TeX 由来は origin=tex_source（原文由来）なので推定 provenance の損失は無い。
        let d = doc(vec![
            node(1, "document", Some("tex_source")),
            node(2, "paragraph", Some("tex_source")),
        ]);
        let ws = collect(&d, &MARKDOWN);
        assert!(
            !codes(&ws).contains(&"inferred_provenance_dropped"),
            "{ws:?}"
        );
    }

    #[test]
    fn pdf_document_warns_about_inferred_provenance_with_origin_breakdown() {
        let d = doc(vec![
            node(1, "document", Some("pdf_text_layer")),
            node(2, "paragraph", Some("layout_model")),
            node(3, "figure", Some("layout_model")),
            node(4, "figure", Some("llm_inference")),
        ]);
        let ws = collect(&d, &MARKDOWN);
        let w = ws
            .iter()
            .find(|w| w.code == ExportWarningCode::InferredProvenanceDropped)
            .expect("{ws:?}");
        assert_eq!(w.count, 3, "pdf_text_layer は原文由来なので数えない");
        assert_eq!(w.detail.as_deref(), Some("layout_model: 2, llm_inference: 1"));
        assert_eq!(w.severity, ExportSeverity::Warn);
    }

    #[test]
    fn relations_symbols_fragments_assets_report_exhaustive_counts() {
        let mut n = node(2, "paragraph", None);
        n.source_fragments = vec![
            LcirFragment {
                page: 1,
                bbox: BBox { x: 0.0, y: 0.0, width: 1.0, height: 1.0 },
                fragment_type: None,
            },
            LcirFragment {
                page: 2,
                bbox: BBox { x: 0.0, y: 0.0, width: 1.0, height: 1.0 },
                fragment_type: None,
            },
        ];
        let mut f = node(3, "figure", None);
        f.assets = vec![LcirAsset {
            role: "page_crop".to_string(),
            mime_type: "image/png".to_string(),
            relative_path: "a.png".to_string(),
            width: None,
            height: None,
            size_bytes: None,
            sha256: "x".to_string(),
            metadata: None,
        }];
        let mut d = doc(vec![node(1, "document", None), n, f]);
        d.relations = vec![
            LcirRelation {
                from_node_id: 2,
                relation_type: "cites".to_string(),
                to_node_id: 3,
                confidence: None,
                origin: None,
                metadata: None,
            },
            LcirRelation {
                from_node_id: 2,
                relation_type: "refers_to_figure".to_string(),
                to_node_id: 3,
                confidence: None,
                origin: None,
                metadata: None,
            },
        ];
        d.symbols = vec![symbol()];

        let ws = collect(&d, &MARKDOWN);
        let by = |c: ExportWarningCode| ws.iter().find(|w| w.code == c).cloned();
        assert_eq!(by(ExportWarningCode::RelationsDropped).unwrap().count, 2);
        assert_eq!(
            by(ExportWarningCode::RelationsDropped).unwrap().detail.as_deref(),
            Some("cites: 1, refers_to_figure: 1"),
            "type 別の内訳を付ける"
        );
        assert_eq!(by(ExportWarningCode::SymbolsDropped).unwrap().count, 1);
        assert_eq!(by(ExportWarningCode::SourceFragmentsDropped).unwrap().count, 2);
        assert_eq!(by(ExportWarningCode::AssetsNotEmbedded).unwrap().count, 1);

        // LCIR JSON は無損失なので、同じ文書でもアセット実体の非同梱だけが残る。
        assert_eq!(codes(&collect(&d, &LCIR_JSON)), vec!["assets_not_embedded"]);
    }

    #[test]
    fn table_with_rowspan_is_reported_as_flattened() {
        let mut t = node(2, "table", None);
        t.payload = Some(serde_json::json!({
            "rows": [
                {"cells": [{"text": "a", "rowspan": 2}, {"text": "b"}]},
                {"cells": [{"text": "c"}]}
            ]
        }));
        let d = doc(vec![node(1, "document", None), t]);
        let ws = collect(&d, &MARKDOWN);
        let w = ws
            .iter()
            .find(|w| w.code == ExportWarningCode::TableRowspanFlattened)
            .expect("{ws:?}");
        assert_eq!(w.count, 1);
        assert_eq!(w.severity, ExportSeverity::Info);
    }

    #[test]
    fn table_without_rowspan_does_not_warn() {
        let mut t = node(2, "table", None);
        t.payload = Some(serde_json::json!({"rows": [{"cells": [{"text": "a"}]}]}));
        let d = doc(vec![node(1, "document", None), t]);
        assert!(!codes(&collect(&d, &MARKDOWN)).contains(&"table_rowspan_flattened"));
    }

    #[test]
    fn warnings_are_sorted_by_severity_then_code() {
        let mut n = node(2, "paragraph", Some("layout_model"));
        n.source_fragments = vec![LcirFragment {
            page: 1,
            bbox: BBox { x: 0.0, y: 0.0, width: 1.0, height: 1.0 },
            fragment_type: None,
        }];
        let mut d = doc(vec![node(1, "document", None), n]);
        d.symbols = vec![symbol()];
        let ws = collect(&d, &MARKDOWN);
        // warn が先・その中は enum の宣言順（安定）。
        assert_eq!(
            codes(&ws),
            vec![
                "symbols_dropped",
                "inferred_provenance_dropped",
                "source_fragments_dropped"
            ],
            "{ws:?}"
        );
        assert!(ws.windows(2).all(|w| w[0].severity <= w[1].severity));
    }
}
