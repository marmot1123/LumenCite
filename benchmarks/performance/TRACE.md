# 実利用のMCP stdioトレース

この変更を含むLumenCiteバイナリの `--mcp-stdio` を使う接続が対象です。
既定はOFFです。既存の接続設定の環境変数に次の2項目を追加し、
クライアント側のstdioプロセスを再起動してください。URL・認証設定はそのまま使います。

```text
LUMENCITE_MCP_TRACE=1
LUMENCITE_MCP_TRACE_DIR=/absolute/private/directory/mcp-traces
```

ディレクトリは実際の絶対パスに置換してください。OFFにするには
`LUMENCITE_MCP_TRACE` を削除するか `0` にしてstdioプロセスを再起動します。
起動済みプロセスには設定変更は反映されません。アプリのGUI設定やDBには保存しません。

セッションごとに新規JSONLを作成します。Unixではファイル0600、
新規ディレクトリ0700です。既存ディレクトリの権限は変更しません。
stdoutはMCP応答専用です。保存先不備・書込失敗・64MiB上限では固定文のstderr通知だけを出し、
トレースを停止して転送を継続します。保存済みファイルは自動削除しません。

## 記録内容と限界

- session: UTC開始時刻、アプリ版、debug/release、プロセスID、タイムアウト設定。
- call_start: 到着順のローカル連番、既知のメソッド/ツール名、要求バイト数、
  許可された数値ID・件数・sourceなど。検索語・引用キー等は除外。
- call_end: HTTP転送時間、stdout処理までの時間、応答サイズ、固定分類の成否とHTTPステータス。

生のRPC ID、認証情報、URL、検索語、本文、エラー本文、未知の名前は保存しません。
数値IDや呼出し時刻も利用履歴なので、トレースは公開リポジトリ外で保管してください。
スキーマの詳細は [API_SPEC](../../docs/API_SPEC.md) を参照してください。

start/endはsession_idとrequest_idで結びます。startだけなら未完了として扱います。
offset_msは同一プロセスの単調時計です。duration_msにはstart記録コストを含みます。
forward_duration_msはその内側のHTTP処理時間であり、両方を加算しません。
応答サイズはデコード後のUTF-8バイト数で、ネットワーク上の圧縮済みサイズではありません。
stdout_bytesは予定サイズであり、書込失敗時の実送信サイズではありません。

直接HTTP接続は対象外です。DB/LCIR内訳、モデルの推論時間、クライアントの待ち行列、
最初の根拠表示・最終回答表示は測りません。呼出し間の空白を推論時間と断定しないでください。
再試行も同一質問かどうかを推測せず、到着した各要求として記録します。
検索・本文取得の実装、返却内容、タイムアウトは変更しません。

## 実際の呼出し列の再計測

トレースは本文も検索語も保存しないため、それだけでは検索を完全再現できません。
replay_complete=falseの呼出しには、別の非公開JSONで省略された引数を補います。
session_id/request_idは対象トレースの値に合わせます。例:

```json
[
  {
    "session_id": "対象セッションID",
    "request_id": 3,
    "arguments": {"query": "quantum walk"}
  }
]
```

記録されたsafe_argumentsと合わせて元の引数一式になるようにしてください。
この入力ファイルと生成ケースには検索語などが入るため、Git管理外に置きます。
変換は成功した読取ツールだけを許可し、書込・未知のツール、失敗・未完了要求、
不足する補完ファイル、記録済み引数との衝突があれば停止します。
補完内容の意味的な正しさまでは検証できません。

```bash
python3 benchmarks/performance/trace_to_cases.py /absolute/private/mcp-trace.jsonl \
  --inputs /absolute/private/inputs.json --output /absolute/private/cases.json
python3 benchmarks/performance/replay_sequence.py --cases /absolute/private/cases.json \
  --output /absolute/private/replay-run --warmup 1 --repeats 5
```

補完不要なら--inputsは省略できます。生成先は新規ファイルに限り、Unixでは0600です。
再生器の接続設定は [README](README.md) に従ってください。
再生では初期化後、読取ツールを元の順序で逐次実行します。元の呼出し間隔、RPC ID、
初期化要求そのもの、モデル処理は再現しません。同じDB・文書バージョンを使い、
比較時はトレースON/OFFとキャッシュ条件も記録してください。
このJSONLを既存のsummarize.pyへ直接入力することはできません。
