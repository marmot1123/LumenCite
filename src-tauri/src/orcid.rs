//! ORCID Public API (`https://pub.orcid.org/v3.0/{id}/person`) からの著者情報取得。
//!
//! v0.3.0 M12 で追加。AuthorEditor の「ORCID から取得」ボタンが呼ぶ。
//! 認証不要 / レート制限は無認証で 24 req/s/IP（Public API）。
//!
//! 取れるもの: given/family/credit-name, emails (public のみ), researcher-urls,
//!             external-identifiers (Scopus / ResearcherID / Loop / Wikidata 等)。
//! 取れないもの: 読み仮名・suffix・name_particle・is_organization は ORCID 仕様に無い。
//! best-effort: other-names に CJK が含まれていれば name_original / original_script を推定。

use serde::Deserialize;

use crate::models::{AuthorIdentifierInput, AuthorInput};

const ORCID_PATTERN_LEN: usize = 19; // "0000-0000-0000-000X"

/// ORCID をフェッチし、AuthorInput に詰めて返す。
///
/// `orcid_raw` は素の id ("0000-...") でも URL ("https://orcid.org/0000-...") でも OK。
/// 内部で末尾の id 部分だけ取り出す（`crate::metadata::normalize_orcid` と同等処理）。
pub async fn fetch_by_orcid(orcid_raw: &str) -> Result<AuthorInput, String> {
    let id = normalize_orcid(orcid_raw);
    if !is_valid_orcid_shape(&id) {
        return Err("ORCID 形式が不正です (0000-0000-0000-000X 形式で入力)".to_string());
    }

    let url = format!("https://pub.orcid.org/v3.0/{}/person", id);
    let client = reqwest::Client::builder()
        .user_agent("LumenCite/0.3 (mailto:support@lumencite.app)")
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client
        .get(&url)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("ネットワークエラー: {}", e))?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err("ORCID が見つかりませんでした".to_string());
    }
    let resp = resp
        .error_for_status()
        .map_err(|e| format!("ORCID API エラー: {}", e))?;

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("レスポンス解析エラー: {}", e))?;

    Ok(person_to_author_input(&body, &id))
}

/// ORCID 値を素の id（"0000-...-000X"）に揃える。URL 形式 / 末尾スラッシュ / 空白を吸収。
fn normalize_orcid(raw: &str) -> String {
    raw.trim()
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("")
        .trim()
        .to_string()
}

fn is_valid_orcid_shape(id: &str) -> bool {
    if id.len() != ORCID_PATTERN_LEN {
        return false;
    }
    // パターン: 4 桁 - 4 桁 - 4 桁 - 4 桁 (末尾は X 可)
    let bytes = id.as_bytes();
    let dashes = [4, 9, 14];
    for (i, b) in bytes.iter().enumerate() {
        if dashes.contains(&i) {
            if *b != b'-' {
                return false;
            }
            continue;
        }
        let is_digit = (*b).is_ascii_digit();
        let is_x_tail = i == ORCID_PATTERN_LEN - 1 && (*b == b'X' || *b == b'x');
        if !is_digit && !is_x_tail {
            return false;
        }
    }
    true
}

/// `/v3.0/{id}/person` JSON を `AuthorInput` に変換する。
///
/// パース部だけを独立関数にすることで、HTTP を叩かずにユニットテスト可能にする。
pub(crate) fn person_to_author_input(body: &serde_json::Value, id: &str) -> AuthorInput {
    let person: Person = serde_json::from_value(body.clone()).unwrap_or_default();

    let given = person.name.as_ref().and_then(|n| value_text(&n.given_names));
    let family = person.name.as_ref().and_then(|n| value_text(&n.family_name));
    let credit = person.name.as_ref().and_then(|n| value_text(&n.credit_name));

    // 表示名: credit-name 優先、無ければ "given family"、両方無ければ id をそのまま
    let display_name = credit
        .clone()
        .or_else(|| match (given.as_deref(), family.as_deref()) {
            (Some(g), Some(f)) => Some(format!("{} {}", g, f)),
            (Some(g), None) => Some(g.to_string()),
            (None, Some(f)) => Some(f.to_string()),
            (None, None) => None,
        })
        .unwrap_or_else(|| id.to_string());

    // middle_name: given-names が複数語なら 2 語目以降を結合（ヒューリスティック）
    let (given_head, middle) = split_middle_name(given.as_deref());

    // public email （複数あれば最初の 1 件）
    let email = person
        .emails
        .as_ref()
        .and_then(|e| e.email.as_ref())
        .and_then(|list| list.first())
        .and_then(|x| x.email.clone())
        .filter(|s| !s.is_empty());

    // researcher-urls の先頭を homepage_url に
    let homepage_url = person
        .researcher_urls
        .as_ref()
        .and_then(|r| r.researcher_url.as_ref())
        .and_then(|list| list.first())
        .and_then(|x| value_text(&x.url));

    // other-names から CJK / Cyrillic / Hangul を含むものを 1 つ拾って name_original にする
    let (name_original, original_script) = person
        .other_names
        .as_ref()
        .and_then(|o| o.other_name.as_ref())
        .and_then(|list| {
            list.iter()
                .filter_map(|x| x.content.as_deref().map(str::trim).filter(|s| !s.is_empty()))
                .find_map(|s| detect_non_latin_script(s).map(|sc| (s.to_string(), sc.to_string())))
        })
        .map(|(n, s)| (Some(n), Some(s)))
        .unwrap_or((None, None));

    // external-identifiers を AuthorIdentifierInput[] に変換
    let identifiers = person
        .external_identifiers
        .as_ref()
        .and_then(|x| x.external_identifier.as_ref())
        .map(|list| {
            list.iter()
                .filter_map(map_external_identifier)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    AuthorInput {
        name: display_name,
        given_name: given_head,
        middle_name: middle,
        family_name: family,
        name_original,
        original_script,
        email,
        homepage_url,
        orcid: Some(id.to_string()),
        identifiers,
        ..Default::default()
    }
}

/// "John F." のような given-names から middle name を切り出す。
/// - 単語 1 つだけ → middle は None
/// - 単語 2 つ以上 → 先頭を given_name、2 語目以降を middle_name にまとめる
fn split_middle_name(given: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(g) = given.map(str::trim).filter(|s| !s.is_empty()) else {
        return (None, None);
    };
    let mut parts = g.split_whitespace();
    let head = parts.next().map(str::to_string);
    let rest: Vec<&str> = parts.collect();
    let middle = if rest.is_empty() {
        None
    } else {
        Some(rest.join(" "))
    };
    (head, middle)
}

/// 文字列に含まれる **非ラテン文字** から ISO 15924 コードを best-effort で 1 つだけ推定。
/// 複数 script が混在しても最初に当たったものを返す（漢字 + ひらがな等）。
fn detect_non_latin_script(s: &str) -> Option<&'static str> {
    for c in s.chars() {
        let cp = c as u32;
        // CJK Unified Ideographs (Hani)
        if (0x4E00..=0x9FFF).contains(&cp)
            || (0x3400..=0x4DBF).contains(&cp)
            || (0x20000..=0x2A6DF).contains(&cp)
        {
            return Some("Hani");
        }
        // Hiragana
        if (0x3040..=0x309F).contains(&cp) {
            return Some("Hira");
        }
        // Katakana
        if (0x30A0..=0x30FF).contains(&cp) {
            return Some("Kana");
        }
        // Hangul Syllables
        if (0xAC00..=0xD7AF).contains(&cp) {
            return Some("Hang");
        }
        // Cyrillic
        if (0x0400..=0x04FF).contains(&cp) {
            return Some("Cyrl");
        }
        // Arabic
        if (0x0600..=0x06FF).contains(&cp) {
            return Some("Arab");
        }
    }
    None
}

/// ORCID の external-identifier-type 文字列を、LumenCite 内部の scheme キー（snake_case）に
/// マッピング。未知の type もそのまま小文字 snake_case に正規化して保持する（捨てない）。
fn map_external_identifier(x: &ExternalIdentifier) -> Option<AuthorIdentifierInput> {
    let raw_type = x.external_id_type.as_deref()?.trim();
    let value = x.external_id_value.as_deref()?.trim();
    if raw_type.is_empty() || value.is_empty() {
        return None;
    }
    let scheme = canonical_scheme(raw_type);
    let url = value_text(&x.external_id_url).filter(|s| !s.is_empty());
    Some(AuthorIdentifierInput {
        scheme,
        value: value.to_string(),
        url,
    })
}

/// "Scopus Author ID" → "scopus" のような既知エイリアスマップ + フォールバック。
fn canonical_scheme(raw: &str) -> String {
    let lower = raw.to_lowercase();
    match lower.as_str() {
        "scopus author id" | "scopus authorid" | "scopus_author_id" => "scopus".to_string(),
        "researcherid" | "researcher id" => "researcher_id".to_string(),
        "semantic scholar" | "semantic_scholar" | "semantic scholar author id" => {
            "semantic_scholar".to_string()
        }
        "google scholar" | "google scholar profile" => "google_scholar".to_string(),
        "loop profile" | "loop" => "loop".to_string(),
        // ORCID / Wikidata / ISNI / VIAF などは小文字化だけで揃う想定
        _ => lower.replace([' ', '-'], "_"),
    }
}

// ─── ORCID JSON 構造（必要部分だけ） ─────────────────────────────────────────

/// `/person` レスポンスの最小サブセット。
/// ORCID のスキーマは何かしらの値を `{ "value": "..." }` で包む癖があるので
/// `Value<String>` を一段挟む。欠けているフィールドはすべて Option で受ける。
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Person {
    name: Option<Name>,
    #[serde(rename = "other-names")]
    other_names: Option<OtherNames>,
    emails: Option<Emails>,
    #[serde(rename = "researcher-urls")]
    researcher_urls: Option<ResearcherUrls>,
    #[serde(rename = "external-identifiers")]
    external_identifiers: Option<ExternalIdentifiers>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Name {
    #[serde(rename = "given-names")]
    given_names: Option<Value<String>>,
    #[serde(rename = "family-name")]
    family_name: Option<Value<String>>,
    #[serde(rename = "credit-name")]
    credit_name: Option<Value<String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct OtherNames {
    #[serde(rename = "other-name")]
    other_name: Option<Vec<OtherName>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct OtherName {
    content: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Emails {
    email: Option<Vec<EmailEntry>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct EmailEntry {
    email: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ResearcherUrls {
    #[serde(rename = "researcher-url")]
    researcher_url: Option<Vec<ResearcherUrl>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ResearcherUrl {
    url: Option<Value<String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ExternalIdentifiers {
    #[serde(rename = "external-identifier")]
    external_identifier: Option<Vec<ExternalIdentifier>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ExternalIdentifier {
    #[serde(rename = "external-id-type")]
    external_id_type: Option<String>,
    #[serde(rename = "external-id-value")]
    external_id_value: Option<String>,
    #[serde(rename = "external-id-url")]
    external_id_url: Option<Value<String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Value<T: Default> {
    value: Option<T>,
}

fn value_text(v: &Option<Value<String>>) -> Option<String> {
    v.as_ref()
        .and_then(|w| w.value.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalize_orcid_handles_url_and_trailing_slash() {
        assert_eq!(normalize_orcid("0000-0002-1825-0097"), "0000-0002-1825-0097");
        assert_eq!(
            normalize_orcid("https://orcid.org/0000-0002-1825-0097"),
            "0000-0002-1825-0097"
        );
        assert_eq!(
            normalize_orcid("http://orcid.org/0000-0002-1825-0097/"),
            "0000-0002-1825-0097"
        );
        assert_eq!(
            normalize_orcid("  0000-0002-1825-0097  "),
            "0000-0002-1825-0097"
        );
    }

    #[test]
    fn validates_orcid_shape() {
        assert!(is_valid_orcid_shape("0000-0002-1825-0097"));
        assert!(is_valid_orcid_shape("0000-0002-1825-009X"));
        assert!(is_valid_orcid_shape("0000-0002-1825-009x"));
        assert!(!is_valid_orcid_shape(""));
        assert!(!is_valid_orcid_shape("0000-0002-1825-009"));
        assert!(!is_valid_orcid_shape("0000-0002-1825-00977"));
        assert!(!is_valid_orcid_shape("0000-0002_1825-0097"));
        assert!(!is_valid_orcid_shape("0000-0002-1825-00ZZ"));
    }

    #[test]
    fn parse_minimal_person_keeps_orcid_and_falls_back_to_given_family() {
        let body = json!({
            "name": {
                "given-names": { "value": "Albert" },
                "family-name": { "value": "Einstein" }
            }
        });
        let input = person_to_author_input(&body, "0000-0002-1825-0097");
        assert_eq!(input.name, "Albert Einstein");
        assert_eq!(input.given_name.as_deref(), Some("Albert"));
        assert_eq!(input.family_name.as_deref(), Some("Einstein"));
        assert!(input.middle_name.is_none());
        assert_eq!(input.orcid.as_deref(), Some("0000-0002-1825-0097"));
        assert!(input.identifiers.is_empty());
    }

    #[test]
    fn parse_person_prefers_credit_name() {
        let body = json!({
            "name": {
                "given-names": { "value": "Albert" },
                "family-name": { "value": "Einstein" },
                "credit-name": { "value": "A. Einstein" }
            }
        });
        let input = person_to_author_input(&body, "0000-0002-1825-0097");
        assert_eq!(input.name, "A. Einstein", "credit-name を優先");
        assert_eq!(input.given_name.as_deref(), Some("Albert"));
        assert_eq!(input.family_name.as_deref(), Some("Einstein"));
    }

    #[test]
    fn parse_middle_name_from_multiword_given() {
        let body = json!({
            "name": {
                "given-names": { "value": "John Fitzgerald" },
                "family-name": { "value": "Kennedy" }
            }
        });
        let input = person_to_author_input(&body, "0000-0000-0000-000X");
        assert_eq!(input.given_name.as_deref(), Some("John"));
        assert_eq!(input.middle_name.as_deref(), Some("Fitzgerald"));
        assert_eq!(input.family_name.as_deref(), Some("Kennedy"));
    }

    #[test]
    fn parse_public_email_and_homepage() {
        let body = json!({
            "name": {
                "given-names": { "value": "A" },
                "family-name": { "value": "B" }
            },
            "emails": { "email": [{ "email": "a@b.org" }] },
            "researcher-urls": {
                "researcher-url": [
                    { "url-name": "Lab", "url": { "value": "https://lab.example.com" } }
                ]
            }
        });
        let input = person_to_author_input(&body, "0000-0000-0000-0001");
        assert_eq!(input.email.as_deref(), Some("a@b.org"));
        assert_eq!(input.homepage_url.as_deref(), Some("https://lab.example.com"));
    }

    #[test]
    fn parse_external_identifiers_with_canonical_schemes() {
        let body = json!({
            "name": {
                "given-names": { "value": "X" },
                "family-name": { "value": "Y" }
            },
            "external-identifiers": {
                "external-identifier": [
                    {
                        "external-id-type": "Scopus Author ID",
                        "external-id-value": "12345678900",
                        "external-id-url": { "value": "https://www.scopus.com/auth?id=12345678900" }
                    },
                    {
                        "external-id-type": "ResearcherID",
                        "external-id-value": "A-1234-2015"
                    },
                    {
                        "external-id-type": "Loop profile",
                        "external-id-value": "999"
                    },
                    {
                        "external-id-type": "Some New Service",
                        "external-id-value": "abc-1"
                    }
                ]
            }
        });
        let input = person_to_author_input(&body, "0000-0000-0000-0001");
        let by_scheme: std::collections::HashMap<&str, &AuthorIdentifierInput> =
            input.identifiers.iter().map(|i| (i.scheme.as_str(), i)).collect();
        assert_eq!(by_scheme.get("scopus").unwrap().value, "12345678900");
        assert_eq!(
            by_scheme.get("scopus").unwrap().url.as_deref(),
            Some("https://www.scopus.com/auth?id=12345678900")
        );
        assert_eq!(by_scheme.get("researcher_id").unwrap().value, "A-1234-2015");
        assert_eq!(by_scheme.get("loop").unwrap().value, "999");
        // 未知タイプも小文字 snake_case で取り込まれる
        assert!(by_scheme.contains_key("some_new_service"));
    }

    #[test]
    fn parse_other_names_detects_cjk_for_name_original() {
        let body = json!({
            "name": {
                "given-names": { "value": "Motoki" },
                "family-name": { "value": "Seki" }
            },
            "other-names": {
                "other-name": [
                    { "content": "Mo Seki" },        // ラテン文字のみ → skip
                    { "content": "関 元樹" },          // CJK → 拾う
                    { "content": "せき もとき" }       // Hiragana
                ]
            }
        });
        let input = person_to_author_input(&body, "0000-0000-0000-0001");
        assert_eq!(input.name_original.as_deref(), Some("関 元樹"));
        assert_eq!(input.original_script.as_deref(), Some("Hani"));
    }

    #[test]
    fn parse_other_names_picks_hangul_when_no_kanji() {
        let body = json!({
            "name": {
                "given-names": { "value": "G" },
                "family-name": { "value": "K" }
            },
            "other-names": {
                "other-name": [{ "content": "김 철수" }]
            }
        });
        let input = person_to_author_input(&body, "0000-0000-0000-0001");
        assert_eq!(input.original_script.as_deref(), Some("Hang"));
    }

    #[test]
    fn parse_drops_external_identifier_with_missing_fields() {
        let body = json!({
            "name": { "given-names": { "value": "X" }, "family-name": { "value": "Y" } },
            "external-identifiers": {
                "external-identifier": [
                    { "external-id-value": "no-type" },
                    { "external-id-type": "Scopus Author ID" }
                ]
            }
        });
        let input = person_to_author_input(&body, "0000-0000-0000-0001");
        assert!(input.identifiers.is_empty());
    }

    #[test]
    fn parse_falls_back_to_orcid_when_no_name() {
        // ORCID では極めて稀だが、name 全欠落ケースでも壊れない
        let body = json!({});
        let input = person_to_author_input(&body, "0000-0000-0000-0001");
        assert_eq!(input.name, "0000-0000-0000-0001");
        assert!(input.given_name.is_none());
        assert!(input.family_name.is_none());
    }

    #[test]
    fn detect_non_latin_script_returns_none_for_pure_latin() {
        assert_eq!(detect_non_latin_script("Albert Einstein"), None);
        assert_eq!(detect_non_latin_script("Café"), None);  // ラテン拡張は対象外
    }
}
