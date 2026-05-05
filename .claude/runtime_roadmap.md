# Runtime Roadmap — micro_xrce_dds_rs v0.1 → v0.4

> 親ドキュメント: [runtime_design.md](runtime_design.md)
> 最終更新: 2026-05-04

## マイルストーン全景

| ver  | 内容                                | 目安       | 状態 |
| ---- | ----------------------------------- | ---------- | ---- |
| 0.1  | 現状 (Session 単独所有モデル)       | 完了       | ✅    |
| 0.2  | Runtime / Context / Node 抽象       | 1〜2 週    | 着手前 |
| 0.3  | Service (Server / Client)           | +1 週      |      |
| 0.4  | Action (Server / Client)            | +1 週      |      |
| 0.5  | Reconnect & resilience              | バッファ   |      |

---

## v0.2 — Runtime layer (大改修)

### Phase 0: 準備 (~半日) ✅ 2026-05-05

- [x] **DESIGN review** — `.claude/runtime_design.md` を読み返し、Section 4 (公開 API) と Section 5 (内部) に同意するメンバーを揃える。
- [x] **wire fixture テスト** — `session.rs` に `#[cfg(test)] mod wire_tests` を追加。`CREATE_CLIENT` / `CREATE_PARTICIPANT (xml="my_node")` / `WRITE_DATA (Float32 1.0)` の 3 ケースを実装。  
  実行: `cargo test --no-default-features` (host target) → 3 passed。  
  *備考: tests/ ディレクトリではなく `#[cfg(test)]` inline に変更 (private 関数へのアクセスが簡単、no_std テスト設定が不要)。*  
  *portable-atomic: `unsafe-assume-single-core` を ARM のみに条件分岐 (`Cargo.toml` 修正)。サブクレートに `.cargo/config.toml` でホストターゲットを指定。*
- [x] **ベースライン計測**: `cargo size-all` (wifi+sensor)  
  - wifi_microros_sensors: text=**209864** data=56 bss=34656 total=244576  
  - default (LED demo): text=13548 data=56 bss=1560 total=15164

### Phase 1: 内部 state の `static` 化 (~1日) ✅ 2026-05-05

- [x] `src/rt/inner.rs` 新設。`SessionInner` に AtomicU8/U16/U32/Bool 群 + creation mailbox (`Mutex<CSRawMutex,()>` + `AtomicU16` + `Signal`) + `Channel<CSRawMutex,Frame,2>` + `subs Mutex` を実装。`const fn new()` あり。
- [x] `src/rt/mod.rs` で `Runtime` (= `SessionInner` の `'static` ホルダ) と `Context` (`&'static SessionInner`) を定義。`RuntimeConfig` (session_id + client_key) も追加。
- [x] `Context: Clone + Copy` を確認 (`#[derive(Clone, Copy)]` + `unsafe impl Send/Sync`)。
- [x] `is_disconnected` / `set_disconnected` / `next_seq` / `next_req` / `alloc_*` / `arm_creation` / `disarm_creation` / `deliver_status` を `SessionInner` の `&self` メソッドに実装。
- [x] `lib.rs` に `pub mod rt` + `pub use rt::{Context, Runtime, RuntimeConfig}` を追加。
- [x] `cargo test --no-default-features` (host): 3 passed, 0 failed。
- [x] `cargo build --features wifi,sensor --example wifi_microros_sensors` (ARM): Finished, 既存 Session API 維持確認。

> **Phase 1 で先取りした Phase 2 内容**: `Frame` 型・`tx_channel`・`FRAME_BUF_SIZE=512` も `inner.rs` に含めた。Phase 2 では `Executor::run()` 内でこれを使うだけ。

### Phase 2+3: TX queue 配線 + Executor タスク化 (~1日) ✅ 2026-05-05

- [x] `Frame` 型 (`bytes: [u8; 512], len: usize`) — Phase 1 で `inner.rs` に定義済み。
- [x] `tx_channel: Channel<CSRawMutex, Frame, 2>` — Phase 1 で `SessionInner` に含まれている。
- [x] `src/rt/executor.rs` で `Executor::run()` を実装。`select(tx_channel.receive(), read_one_frame(...))` で多重化。  
  embassy-futures 0.1 を追加。
- [x] dispatch 関数 (旧 `Session::dispatch_frame`) を module-level `dispatch_frame` / `dispatch_data` に移植。
- [x] STATUS 受信時に `deliver_status(req_id, result)` を呼び出す。`STATUS_OK_MATCHED` は warn + `Ok(())` で続行。
- [x] disconnect 検出時: `set_disconnected()` + `creation_signal.signal(Err(Disconnected))` + `pending::<()>().await` でハング。
- [x] `Runtime::start<T: Read+Write>(&'static self, transport, config) -> Result<(Context, Executor<T>), Error>` を実装。  
  *備考: `start()` は `(Context, Executor<T>)` を返す。spawner 呼び出しはユーザー main 側に委譲 (embassy-executor を crate 依存から除外するため)。*  
  *`session::build_create_client` と `parse_status_agent` を `pub(crate)` に昇格。*
- [x] `cargo test --no-default-features` (host): 3 passed, 0 failed。
- [x] `cargo build --features wifi,sensor --example wifi_microros_sensors` (ARM): Finished, 既存 Session API 維持確認。

### Phase 4: 公開 API (`Node` / `Publisher` / `Subscription`) 移植 (~1〜2日)

- [ ] `src/rt/encode.rs` に旧 `session.rs` の `encode_create_*` / `encode_read_data` / `finalize_write_data_headers` 系を **非 generic 関数** として移管。
- [ ] `src/rt/creation.rs` に `send_create_and_wait(ctx, req_id, build_fn)` を実装。
- [ ] `src/node.rs` を書き直し: `Node { ctx, participant_idx, publisher_idx, subscriber_idx }` + `create_publisher` / `create_subscription` を `Context::create_node` 経由に。
- [ ] `src/publisher.rs` の `Publisher<M>::publish` を、generic shim → 非 generic `finalize_and_send` 呼び出しに変更。
- [ ] `src/subscription.rs` に `subscription_slot!` macro を追加 (現状 `StaticCell` が必要だったのを `static` 1 行に簡略化)。
- [ ] `src/lib.rs` から旧 `Session` re-export を削除し、`Runtime` / `Context` / `Node` / `Publisher` / `Subscription` を公開。

### Phase 5: 旧 `session.rs` の削除 + 例の書き換え (~1日)

- [ ] `src/session.rs` を削除 (中身は分割移管済み)。
- [ ] `examples/microros_hello.rs` を新 API で書き直し (1 main + 1 hello_node task)。  
  *目標: `static Channel<...>` ゼロ。`Session::*` 呼び出しゼロ。*
- [ ] `examples/microros_subscriber.rs` を新 API で書き直し (1 task = 1 node = 1 sub)。
- [ ] `examples/wifi_microros_sensors.rs` を新 API で書き直し:
  - [ ] `bme_node` task — temp/humi/pres を 1 task で publish
  - [ ] `mq3_node` task — ethanol publish
  - [ ] `range_node` task — HC-SR04 publish
  - [ ] `imu_node` task — IMU publish
  - [ ] `static *_CH` 全廃。

### Phase 6: 検証 (~半日)

- [ ] Phase 0 で作った wire fixture テストが green。
- [ ] `cargo size-wifi` / `cargo size-default` を取り、現状値と +/- を README に記録 (目標: ROM 増分 ±0)。
- [ ] 実機で 3 例すべてを flash → `ros2 topic echo` で動作確認。`docker compose up -d` を前提。
- [ ] `cargo doc --open` で公開 API doc が読みやすいか確認。

### v0.2 受け入れ基準 (Definition of Done)

1. `examples/*.rs` の中に `Session` / `static Channel<f32, ...>` という文字列が無い。
2. すべての例が「main で `Runtime::start` → user タスクを spawn」のパターンに統一。
3. wire fixture テストが green。
4. `wifi_microros_sensors` 例で `cargo size` の text が v0.1 比で ±2KB 以内。

---

## v0.3 — Service support (Requester / Replier)

### 前提
v0.2 完了。`Context` 経由のすべての I/O が動いている。

### タスク

- [ ] `src/service.rs` 新設。`Service` トレイト (`type Request / type Response / TYPE_NAME`) を定義。
- [ ] `src/rt/encode.rs` に `encode_create_requester` / `encode_create_replier` を追加 (`OBJK_REQUESTER = 0x07`, `OBJK_REPLIER = 0x08`、XML スキーマは [xrce_dds_protocol.md](xrce_dds_protocol.md) 参照)。
- [ ] `SampleIdentity` (12 bytes: writer GUID + i64 seq_num) のシリアライズを `cdr.rs` に追加。
- [ ] `ServiceRequestSlot<S>` (subscription slot 風 inbox + sample identity 保持) を実装。
- [ ] `ServiceServer<S>::recv_request()` / `ServiceRequest<S>::reply(...)` を実装。
- [ ] `ServiceClient<S>::call()` を実装。  
  内部: 送信前に `pending_calls: Map<SequenceNumber, Signal<Response>>` 風 mailbox に登録、reply 受信で signal。
- [ ] dispatcher 拡張: `SUBMSG_DATA` 受信時、subscription / service-reply / action のどれにディスパッチするかを slot type で判定。
- [ ] サンプル: `examples/microros_service_server.rs` (`add_two_ints`)、`examples/microros_service_client.rs`。

### v0.3 受け入れ基準

1. `ros2 service call /add_two_ints example_interfaces/srv/AddTwoInts '{a: 3, b: 4}'` が `7` を返す。
2. ServiceClient から `call()` した結果が ServiceServer に届き、reply が client に正しく届く。

---

## v0.4 — Action support

### タスク

- [ ] `src/action.rs` 新設。`Action` トレイトとビルトイン service 型 (`SendGoal<A>` / `GetResult<A>` / `CancelGoal`)、topic 型 (`Feedback<A>` / `GoalStatusArray`) を定義。
- [ ] `ActionServer<A>` を 3 service + 2 publisher の合成として実装。
- [ ] `GoalHandle<A>::publish_feedback / succeed / cancel` を実装。
- [ ] `ActionClient<A>::send_goal(...).await` を実装 (`SendGoal` service 呼び出し → `GetResult` の async wait → `Feedback` の stream)。
- [ ] サンプル: `examples/microros_action_server.rs` (`fibonacci`)、`examples/microros_action_client.rs`。

### v0.4 受け入れ基準

1. ROS2 純正 action client (`ros2 action send_goal /fibonacci ...`) が成功する。
2. feedback トピックが purge されず受信できる。

---

## v0.5 — Reconnect & resilience (将来課題)

- TCP 切断時、Runtime が socket を再生成して executor を resume できるようにする。
- 切断中の publish 呼び出しは `Error::Disconnected` を返し続け、reconnect 完了で自動的に通る。
- entity の再 CREATE は不要 (agent は client_key 単位で entity を保持) ⇒ session resume で十分。
- WiFi 切断・DHCP リース更新と組み合わせた E2E 復旧テスト。

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

| シナリオ                           | 現状 v0.1     | 目標 v0.2   |
| ---------------------------------- | ------------- | ----------- |
| `wifi_microros_sensors` ROM (text) | **209,864 B** | ±2 KB       |
| `wifi_microros_sensors` RAM (data+bss) | **34,712 B** | +1 KB 程度 |
| `default` (LED demo) ROM (text)    | 13,548 B      | —           |
| publish レイテンシ (1 msg)         | 未計測        | ≤ 5 ms      |
| publish スループット (Float32)     | 未計測        | ≥ 50 Hz/topic |

(v0.1 ベースラインは 2026-05-05 `cargo size --features wifi,sensor --example wifi_microros_sensors --release -- -B` で取得。)
