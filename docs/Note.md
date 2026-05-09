## Note

### STATUS_OK_MATCHED 警告について
結論: 機能的には完全に正常で、許容して OK。実害はありません。
何が起きているか
XRCE-DDS プロトコルでは agent が CREATE のレスポンスとして 2種類の OK を返します:

ステータス	意味
STATUS_OK	新規にエンティティを作成した
STATUS_OK_MATCHED	同じ client_key + OID + XML のエンティティが既に存在したので、それを そのまま再利用 した
STATUS_OK_MATCHED が出る条件は、典型的には：

同じファームウェア（同じ client_key!()）を再起動した
agent はまだ前回のエンティティを覚えている
これは Fast-DDS の正常な「冪等な作成」の挙動です。

解消する方法
方法	効果	副作用
docker compose down && up -d で agent 完全再起動	警告が消える	再起動するたびに必要
client_key!() を毎回ランダムに変更	警告が消える	agent 側で entity が累積（メモリリーク的）
警告メッセージのログレベルを warn! → debug! に下げる	通常運用でノイズ消える	真の問題が起きた時に気付きにくい
何もしない（許容）	—	警告が出続ける
推奨
何もしないか、debug! に下げるのが現実的です。STATUS_OK_MATCHED 自体は agent の正しい動作で、後続の CREATE が 本当に 失敗するときだけ問題が顕在化します（その場合は別途 AgentRejected(0x80) 等のエラーが出る）。

ログレベルを下げたい場合は変更可能です（executor.rs の該当行を warn! → debug! に変えるだけ）。希望があれば修正します。

