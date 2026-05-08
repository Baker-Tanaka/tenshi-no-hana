# Runtime Roadmap — micro_xrce_dds_rs v0.1 → v0.4

> 親ドキュメント: [runtime_design.md](runtime_design.md)

---
### 残作業

- [ ] 実機 E2E テスト: WiFi 切断 / DHCP リース更新 / agent 再起動 のシナリオで `RUNTIME.resume` がエンティティ再作成なしで通ることを確認。
- [ ] `Publisher::publish` 中に `Disconnected` を捕捉した場合に caller がリトライしやすいよう、`call/publish` の戻り値ドキュメント整備。
- [ ] (オプション) `Executor::run` の disconnect ループから自動 reconnect (transport factory closure を保持) を提供する高レベル API。現状は手動 supervisor。
- [ ] (オプション) buffer pool を unbounded ring (StaticCell ではなく lifetime-managed bump arena) に置き換えて、何回でもリコネクト可能にする。
- [ ] **マルチゴール ActionServer**: 現状 single-flight 前提 → 同時複数ゴール処理対応 (active_goal の AtomicCancelState を複数スロット化、`accept_next_goal` を queue 化)。
- [ ] **実機 E2E 検証**: `microros_action_client.rs` (Fibonacci client) と `microros_action_server.rs` (Fibonacci server) を host の `examples_rclpy_minimal_action_*` に対して動作させ、wire ログで goal_id / SampleIdentity / cancel routing の往復を確認。

---

## 既知の慎重ポイント (Known Pitfalls — v0.2 実装時に踏みうる)

1. **`zerocopy_channel::Sender` は `!Sync`** — `Mutex<...>` でラップする際 `embassy_sync::mutex::Mutex` (非同期 Mutex) を使うこと。`blocking_mutex` ではない。
2. **CREATE 連発時の req_id wraparound** — `next_req` は `u16::wrapping_add(1).max(1)` で 0 を避ける (現コードの慣習を維持)。
3. **`Subscription` 登録は `create_subscription` の中だけ** — `&'static dyn SubscriptionSlot` を `subs` HVec に push する箇所はそこ 1 箇所に集約。複数タスクから push されない (CREATE は creation_lock で直列化されているため自動的に保証される)。
4. **`creation_signal.reset()` のタイミング** — CREATE_* 送信 *前* に reset。送信後に signal が来てから reset すると次の CREATE が古い signal を拾う恐れ。
5. **`with_timeout(creation_signal.wait())` の側で `creation_pending_req` を 0 に戻す** — タイムアウトしたまま放置すると次の CREATE と混同する。
6. **`Frame::bytes` のサイズ決定** — XML の topic XML が ~320 bytes、CDR body の最大値が `M::MAX_SERIALIZED_SIZE`。`std_msgs/String` で 256 bytes ペイロードまで使うなら 384 では不足。`FRAME_BUF_SIZE = 512` も検討。
7. **Subscription overflow → log のみ** — `try_send` 失敗時に publish 側でやることは無い (受信側のキュー溢れ)。`debug!` で済ませる。

---

## ベンチマーク目標値 (参考)

| シナリオ                               | 現状 v0.1     | 目標 v0.2     |
| -------------------------------------- | ------------- | ------------- |
| `wifi_microros_sensors` ROM (text)     | **209,864 B** | ±2 KB         |
| `wifi_microros_sensors` RAM (data+bss) | **34,712 B**  | +1 KB 程度    |
| `default` (LED demo) ROM (text)        | 13,548 B      | —             |
| publish レイテンシ (1 msg)             | 未計測        | ≤ 5 ms        |
| publish スループット (Float32)         | 未計測        | ≥ 50 Hz/topic |

(v0.1 ベースラインは 2026-05-05 `cargo size --features wifi,sensor --example wifi_microros_sensors --release -- -B` で取得。)
