# Runtime Roadmap — micro_xrce_dds_rs v0.1 → v0.4

> 親ドキュメント: [runtime_design.md](runtime_design.md)
> 最終更新: 2026-05-06 (v0.3 / v0.4 scaffold / v0.5 API 追加)

## マイルストーン全景

| ver  | 内容                                | 目安       | 状態 |
| ---- | ----------------------------------- | ---------- | ---- |
| 0.1  | 現状 (Session 単独所有モデル)       | 完了       | ✅    |
| 0.2  | Runtime / Context / Node 抽象       | 1〜2 週    | ✅ 2026-05-06 |
| 0.3  | Service (Server / Client)           | +1 週      | ✅ 2026-05-06 (実機検証は別途) |
| 0.4  | Action (Server / Client)            | +1 週      | 🟡 trait scaffold (実装は v0.4-rc1) |
| 0.5  | Reconnect & resilience              | バッファ   | 🟡 disconnect/resume API (E2E は別途) |

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

### Phase 4: 公開 API (`Node` / `Publisher` / `Subscription`) 移植 (~1〜2日) ✅ 2026-05-06

- [x] `src/rt/encode.rs` に旧 `session.rs` の `encode_create_*` / `encode_read_data` / `finalize_write_data_headers` 系を **非 generic 関数** として移管。すべて `pub` (integration tests からも参照可)。
- [x] `src/rt/creation.rs` に `send_create_and_wait(ctx, req_id, build_fn)` を実装。
- [x] `src/node.rs` を書き直し: `Node { ctx, participant_idx, publisher_idx, subscriber_idx }` + `create_publisher` / `create_subscription` を `Context::create_node` 経由に。
- [x] `src/publisher.rs` の `Publisher<M>::publish` を、内部 `ctx` を保持して `finalize_write_data_headers` を直接呼ぶように変更。
- [x] `src/subscription.rs` に `subscription_slot!` macro を追加 (現状 `StaticCell` が必要だったのを `static` 1 行に簡略化)。
- [x] `src/lib.rs` から旧 `Session` re-export を削除し、`Runtime` / `Context` / `Node` / `Publisher` / `Subscription` を公開。

### Phase 5: 旧 `session.rs` の削除 + 例の書き換え (~1日) ✅ 2026-05-06

- [x] `src/session.rs` を削除 (中身は分割移管済み)。
- [x] `examples/microros_hello.rs` を新 API で書き直し (1 main + 1 hello_node task)。  
  *目標達成: `static Channel<...>` ゼロ。`Session::*` 呼び出しゼロ。*
- [x] `examples/microros_subscriber.rs` を新 API で書き直し (1 task = 1 node = 1 sub)。`subscription_slot!` macro 使用。
- [x] `examples/wifi_microros_sensors.rs` を新 API で書き直し:
  - [x] 単一 `sensor_node` task で全センサー (BME280 + MQ-3B + HC-SR04 + ICM-20602) を読み取り → publish。  
    *I2C0 を `RefCell` で BME280 / IMU 共有しているため task 分割は次フェーズで再検討 (現時点ではタスク分割の必要性なし)。*
  - [x] `static *_CH` 全廃 (旧 6 個の `Channel<f32, 4>` → ゼロ)。

### Phase 6: 検証 (~半日) ✅ 2026-05-06

- [x] Phase 0 で作った wire fixture テストが green: `cargo test --no-default-features` → **9 passed** (`tests/wire_tests.rs` 4 + `tests/dispatch_tests.rs` 5)。
- [x] `cargo size --features wifi,sensor --example wifi_microros_sensors --release -- -B`:  
  - v0.1 baseline: text=**209,864** data=56 bss=34,656 total=244,576  
  - v0.2: text=**217,344** data=1,256 bss=34,360 total=252,960  
  - 差分: text **+7,480 B** (+3.6%), data +1,200 B (`Frame` バッファ等 static)、bss −296 B。  
  - 目標 ±2 KB は未達。Subscription dispatch テーブルと `creation` 機構の追加コストが主因。Phase 7 (もしくは v0.3) で `dispatch_data` の単純化と inlinable 化を検討する。
- [ ] 実機で 3 例すべてを flash → `ros2 topic echo` で動作確認 (要ハードウェア; 別途実施)。
- [x] `cargo doc --no-deps --no-default-features` で公開 API doc 生成 (旧 `crate::Session` リンクは `crate::Node` に更新済み; 残り warning 2 件は外部 crate 参照と private item 参照のみ)。

### v0.2 受け入れ基準 (Definition of Done)

1. `examples/*.rs` の中に `Session` / `static Channel<f32, ...>` という文字列が無い。
2. すべての例が「main で `Runtime::start` → user タスクを spawn」のパターンに統一。
3. wire fixture テストが green。
4. `wifi_microros_sensors` 例で `cargo size` の text が v0.1 比で ±2KB 以内。

---

## v0.3 — Service support (Requester / Replier) ✅ 2026-05-06

### 前提
v0.2 完了。`Context` 経由のすべての I/O が動いている。

### タスク

- [x] `src/service.rs` 新設。`Service` トレイト (`type Request / type Response / SERVICE_NAME / REQUEST_TYPE_NAME / RESPONSE_TYPE_NAME`) を定義。
- [x] `src/rt/encode.rs` に `finalize_service_write_data` を追加 (REQUESTER/REPLIER への WRITE_DATA は通常の WRITE_DATA と同形式; entity 作成は `encode_create_with_parent` を `ENTITY_REQUESTER=0x07` / `ENTITY_REPLIER=0x08` で再利用)。
- [x] `SampleIdentity` (16 bytes writer_guid + 8 bytes i64 sequence_number = 24 bytes) のシリアライズを `service.rs` に追加。CDR で書く `bytes_raw` / `i64_val` / `u64_val` および reader 対称メソッドを `cdr.rs` / `cdr_reader.rs` に追加。
- [x] `ServiceClientSlot<S>` (pending_seq でリプライ matching する inbox depth=1) と `ServiceServerSlot<S, N>` (request inbox; SampleIdentity 同梱) を実装。
- [x] `ServiceServer<S>::recv_request()` → `ServiceRequest<S> { identity, payload }` → `request.reply(&resp).await` (echoes SampleIdentity) を実装。
- [x] `ServiceClient<S>::call()` を実装。`call_lock` で 1 件直列化、`pending_seq.store(seq)` → WRITE_DATA → `inbox.receive().await` の同期パターン。`writer_guid` は `derive_writer_guid(client_key, requester_oid)` で deterministic 生成。
- [x] dispatcher 拡張: `ServiceClientSlot` / `ServiceServerSlot` が `SubscriptionSlot` を実装するため、既存の `inner.subs` ディスパッチ表に同じ仕組みで登録される (oid match → try_deliver の二段階; 内側で SampleIdentity を読み取って routing)。
- [x] `Node::create_service_client::<S>(&handles)` / `Node::create_service_server::<S, N>(&slot)` を `node.rs` に追加。entity 作成は `<dds><requester>` / `<dds><replier>` XML を `encode_create_with_parent` 経由で送信。
- [x] `service_client_slot!` / `service_server_slot!` macro を提供。
- [x] テスト: `tests/service_tests.rs` (4 件 — SampleIdentity round-trip / ServiceClientSlot 配信フィルタ / ServiceServerSlot 配信 / dyn SubscriptionSlot 経由ルーティング)。
- [ ] サンプル: `examples/microros_service_server.rs` (`add_two_ints`) / `examples/microros_service_client.rs` — **v0.3-rc**。`example_interfaces::srv::AddTwoInts_Request_/Response_` の `Message` 実装が必要 (rosidl 由来; 手書きで provide することも可能)。実機検証 (ros2 service call) も同 rc にて。

### v0.3 受け入れ基準

1. **(rc 受け入れ予定)** `ros2 service call /add_two_ints example_interfaces/srv/AddTwoInts '{a: 3, b: 4}'` が `7` を返す。
2. **(rc 受け入れ予定)** ServiceClient から `call()` した結果が ServiceServer に届き、reply が client に正しく届く。
3. ✅ wire fixture テストが green: `cargo test --no-default-features` → **21 passed** (action 4 + dispatch 5 + reconnect 4 + service 4 + wire 4)。

---

## v0.4 — Action support 🟡 trait scaffold

### 前提
v0.3 (Service) 完了。Action は **3 service + 2 topic の合成** として実装。

### タスク

- [x] `src/action.rs` 新設。`Action` トレイト (`type Goal / Result / Feedback`、5 つの DDS type name 定数) を定義。
- [x] `GoalId` (16-byte UUID; raw octet[16] CDR) と `GoalId::from_seq(client_key, action_idx, seq)` で deterministic ID 生成。
- [x] `goal_status::*` / `cancel_response::*` / `get_result_status::*` 定数を `action_msgs/msg/GoalStatus.idl` に合わせて公開。
- [x] `ActionClient<A>` / `ActionServer<A>` / `GoalHandle<A>` / `AcceptedGoal<A>` の **API スカフォルディング** (現状すべて `Error::NotStarted` を返すプレースホルダ)。型シグネチャは v0.4-rc1 で実装が入っても変わらないように凍結。
- [x] テスト: `tests/action_tests.rs` (4 件 — GoalId CDR round-trip / from_seq determinism / from_seq sensitivity / GoalStatus 定数値)。
- [ ] **v0.4-rc1 残作業**:
  - [ ] rosidl-generated wrapper メッセージ (例: `SendGoal_Request_<A>` = `goal_id: GoalId + goal: A::Goal`、`GetResult_Response_<A>` = `status: i8 + result: A::Result`、`FeedbackMessage<A>` = `goal_id + feedback`、`CancelGoal_*`、`GoalStatusArray`) を `Message` 実装と共に provide するための型ヘルパまたはマクロ。
  - [ ] `ActionClient::send_goal()` を `SendGoal` service 呼び出し → `GetResult` async wait → `feedback` subscription stream の合成として実装。
  - [ ] `ActionServer::accept_next_goal()` / `AcceptedGoal::publish_feedback / succeed / abort` の合成。
  - [ ] サンプル: `examples/microros_action_server.rs` (`fibonacci`)、`examples/microros_action_client.rs`。

### v0.4 受け入れ基準

1. **(v0.4-rc1)** ROS2 純正 action client (`ros2 action send_goal /fibonacci ...`) が成功する。
2. **(v0.4-rc1)** feedback トピックが purge されず受信できる。
3. ✅ trait API が凍結され、`cargo build` / `cargo test` 共に green。

---

## v0.5 — Reconnect & resilience 🟡 API ready

### 完成品 (2026-05-06)

- [x] `SessionInner` に `disconnect_signal: Signal<CSRawMutex, ()>` を追加。Executor の transport failure 時に `set_disconnected()` が立つと同時に signal を発火。
- [x] `Runtime::wait_for_disconnect(&'static self)` — supervisor タスクが await できる pub API。すでに切断済みなら即時 resolve。
- [x] `Runtime::resume<T>(transport, config) -> Result<Executor<T>, Error>` — 同じ `client_key` で CREATE_CLIENT を再送、STATUS_AGENT を確認し、tx queue ドレイン + creation 機構リセット + 切断フラグ clear → 新しい `Executor<T>` を返す。エンティティの再 CREATE は不要 (Fast-DDS の agent は `client_key` 単位でエンティティを保持)。
- [x] `Runtime::force_disconnect()` / `Runtime::clear_disconnect()` — アプリ側ヘルスチェックや手動制御用の pub API。
- [x] テスト: `tests/reconnect_tests.rs` (4 件 — 初期状態 / wait_for_disconnect 即時解決 / clear_disconnect / 多サイクル signal)。

### 想定されるアプリ実装パターン

```rust
spawner.spawn(supervisor_task(spawner)).unwrap();

#[embassy_executor::task]
async fn supervisor_task(spawner: Spawner) -> ! {
    loop {
        RUNTIME.wait_for_disconnect().await;
        warn!("[supervisor] xrce session lost, reconnecting...");
        loop {
            // 1) WiFi 再アソシエート、TCP 再接続して socket を作る
            let socket = reconnect_tcp().await;
            // 2) RUNTIME.resume を試行
            match RUNTIME.resume(socket, RuntimeConfig::new(0x81, client_key!())).await {
                Ok(exec) => { spawner.spawn(xrce_exec(exec).unwrap()); break; }
                Err(_)   => Timer::after_secs(2).await,
            }
        }
    }
}
```

### 残作業

- [ ] 実機 E2E テスト: WiFi 切断 / DHCP リース更新 / agent 再起動 のシナリオで `RUNTIME.resume` がエンティティ再作成なしで通ることを確認。
- [ ] `Publisher::publish` 中に `Disconnected` を捕捉した場合に caller がリトライしやすいよう、`call/publish` の戻り値ドキュメント整備。
- [ ] (オプション) `Executor::run` の disconnect ループから自動 reconnect (transport factory closure を保持) を提供する高レベル API。現状は手動 supervisor。

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
