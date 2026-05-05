# micro_xrce_dds_rs ランタイム再設計 — "Embassy task = ROS2 node"

> 作成日: 2026-05-04 / 対象: `external/micro_xrce_dds_rs` v0.1 → v0.2
> 関連: [xrce_dds_protocol.md](xrce_dds_protocol.md), [serialization.md](serialization.md)

---

## 1. 動機 (Motivation)

現状 (`v0.1`) では `Session<T>` が socket・送信バッファ・受信バッファ・サブスク表を一つの構造体に抱え、`&mut self` を要求する。結果として:

- **タスク跨ぎが手動** — `wifi_microros_sensors.rs` ではユーザーが `static Channel<f32, 4>` を 6 本宣言し、センサータスク → microros タスクへ値を流している。Publisher は publish 用のタスクにロックされ、他タスクからは触れない。
- **session が露出** — 例の本体に `Session::connect / create_node / create_publisher / publish / spin_once` の呼び出しが直接書かれる。トランスポート再接続も例ごとに手書き。
- **task = node の対応関係が不在** — ROS2 では「ノード ≒ async コンテキスト」だが、現状は「session を持つ 1 タスク + 雑多な worker」のモノリシック構造。

> 評価: コア (`framing` / `protocol` / `cdr` / `message` / `subscription::SubscriptionSlot`) の作りは良い。問題は **公開 API レイヤーが Embassy の並行モデルと噛み合っていない** こと。

## 2. ゴール (Design Goals)

1. **Task = Node**: 1 つの `#[embassy_executor::task]` が 1 つの ROS2 Node を所有する。タスクは `ctx` を受け取り、自分で `create_node` → `create_publisher` / `create_subscription` する。
2. **Session 隠蔽**: ユーザーコードに `Session` / TCP socket / `spin*()` を一切登場させない。露出するのは `Context` + `Node` + `Publisher<M>` + `Subscription<M, N>` (+ 将来 `Service*` / `Action*`)。
3. **Publisher は cheap-Copy で task-shareable**: `&Publisher<M>` ではなく値で複数タスクに渡せる。`publish().await` は内部 mutex/queue 経由でシリアル化される。
4. **Subscription は `&'static` slot 受信**: 既存の `Subscription<M, N>` パターンを維持。タスクは `slot.recv().await` するだけ。
5. **MCU リソースに優しい**:
   - TX バッファ・RX バッファは **シングルトン** (タスク数に比例しない)。
   - 公開 API の generic 使用箇所は最小化 (`publish<M>` などのモノモルフィズム爆発を避け、シリアライズ後は非 generic な内部関数に流す)。
   - Heap 不可、`heapless` のみ。
6. **将来拡張余地**: Service (Requester / Replier)、Action (3 service + 2 topic) を後付けで追加可能な抽象。
7. **Reconnect は v2 へ**: TCP 切断時の挙動は v1 では「全 publish が `Error::Disconnected` を返す + executor タスクは log してそのまま停止」。再接続は v2 で transport の差し替えとして扱う。

## 3. アーキテクチャ概観

```
┌────────────── User application (no_std, embassy) ──────────────┐
│                                                                │
│  ┌─ task imu_node(ctx) ─────────┐  ┌─ task cmdvel_node(ctx) ─┐ │
│  │ node = ctx.create_node(..)   │  │ node = ctx.create_node()│ │
│  │ p = node.create_publisher()  │  │ s = node.create_subscr()│ │
│  │ loop { p.publish(&imu).await}│  │ loop { s.recv().await } │ │
│  └──────────────┬───────────────┘  └────────────┬────────────┘ │
│                 │ (Copy, Send)                  │              │
└─────────────────┼───────────────────────────────┼──────────────┘
                  │                               │
                  ▼                               ▼
        ┌────────────────────────────────────────────────┐
        │  Context = &'static SessionInner               │
        │  ─ tx_queue (zerocopy_channel<Frame, 2>)       │
        │  ─ creation mailbox (req_id ↔ Signal)          │
        │  ─ counters (seq, req_id, idx alloc)           │
        │  ─ subscription dispatch table                 │
        └──────────────┬─────────────────────────────────┘
                       │
                       ▼
        ┌────────────── Executor (1 task, sole socket owner) ──────────────┐
        │   loop {                                                         │
        │     select(                                                      │
        │       frame = tx_queue.recv() → write_framed(socket, &frame),    │
        │       len   = read_one_frame(socket, &rx_buf) → dispatch(...))   │
        │   }                                                              │
        └────────────────────────┬─────────────────────────────────────────┘
                                 │ embassy-net TcpSocket
                                 ▼
                         (micro-ROS Agent)
```

**3 つの分離**:

- **Tasks (user)** — ロジックだけ書く。I/O は `Context` 越しに send/recv するだけ。
- **Executor (1 task)** — TCP socket の唯一の所有者。`select` で send/recv を多重化する。
- **Inner state (static)** — counters / dispatch table / queues。`AtomicU16` と `embassy_sync::Mutex` のみで保護。

## 4. 公開 API

### 4.1 起動 (boot)

```rust
use micro_xrce_dds_rs::{ros2, Runtime, RuntimeConfig};

// 1. 静的 state を確保
static RUNTIME: Runtime = Runtime::new();

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    // ... wifi/net 起動 (既存通り) ...

    // 2. Agent に接続して executor タスクを spawn
    let ctx = RUNTIME
        .start(stack, agent_endpoint, RuntimeConfig::default(), &spawner)
        .await
        .unwrap();

    // 3. user task を spawn — node はタスク内で自分で作る
    spawner.spawn(imu_node(ctx)).unwrap();
    spawner.spawn(cmdvel_node(ctx)).unwrap();
    spawner.spawn(env_node(ctx)).unwrap();
}
```

`Runtime::start()` が内部で:
- `Session::connect` 相当の CREATE_CLIENT / STATUS_AGENT 往復
- `Executor` タスクを spawn
- `Context` (≈ `&'static SessionInner`) を返す

`Context` は `Copy + Send + Sync` — 任意個のタスクに値で渡せる。

### 4.2 ノードタスク

```rust
use micro_xrce_dds_rs::{Context, ros2::msg, subscription_slot};

// 既存パターン (Subscription::new() は const fn なので static で OK)
subscription_slot!(static SUB_CMDVEL: msg::geometry_msgs::Twist, depth = 4);

#[embassy_executor::task]
async fn imu_node(ctx: Context) -> ! {
    let node = ctx.create_node("imu").await.unwrap();
    let pub_imu  = node.create_publisher::<msg::sensor_msgs::Imu>("/imu").await.unwrap();
    let sub_cmd  = node.create_subscription("/cmd_vel", &SUB_CMDVEL).await.unwrap();

    let mut tick = Ticker::every(Duration::from_millis(20));
    loop {
        if let Some(twist) = sub_cmd.try_recv() { /* update setpoint */ }
        let imu = read_imu();
        let _ = pub_imu.publish(&imu).await;
        tick.next().await;
    }
}
```

### 4.3 型一覧

| 型                        | コピー可 | 役割                                                    |
| ------------------------- | -------- | ------------------------------------------------------- |
| `Runtime`                 | no       | `static` に置く SessionInner ホルダ                     |
| `Context`                 | yes      | `&'static SessionInner` の薄いラッパ。タスクの入口      |
| `Node`                    | yes      | participant/pub/sub idx を保持。`Context` を内蔵        |
| `Publisher<M>`            | yes      | dw_id + `Context`。値コピーで複数タスクに渡せる         |
| `Subscription<M, N>`      | no       | `&'static` slot。`subscription_slot!()` で宣言          |
| `Service<S> / Client<S>`  | yes      | (v0.3) requester / replier ハンドル                     |
| `ActionServer<A> / ActionClient<A>` | yes | (v0.4) 3 service + 2 topic コンポジション         |

すべての I/O API は `async fn` で、その内部実装は **executor タスク経由** の中継のみ — ユーザータスクは TCP に直接触れない。

## 5. 内部設計

### 5.1 `Runtime` / `SessionInner`

```rust
pub struct Runtime {
    inner: SessionInner,  // 'static
}

struct SessionInner {
    // 識別子・カウンタ (atomic)
    session_id: AtomicU8,           // 0 = 未起動, 起動後は 0x81 など
    client_key: AtomicU32,
    seq:                AtomicU16,
    req_id:             AtomicU16,
    next_participant:   AtomicU16,
    next_topic:         AtomicU16,
    next_publisher:     AtomicU16,
    next_subscriber:    AtomicU16,
    next_dw:            AtomicU16,
    next_dr:            AtomicU16,

    // CREATE_* の STATUS 待ち合わせ用 mailbox (同時に 1 件しか通さない)
    creation_lock: Mutex<CSRawMutex, ()>,                  // serialize CREATE
    creation_pending_req: AtomicU16,                       // 0 = 無し
    creation_signal: Signal<CSRawMutex, Result<(), Error>>,

    // 送信キュー (zerocopy ring buffer of frames)
    tx_send: ...,  // Sender side, 内部 mutex で複数 producer 共有
    // ↑ zerocopy_channel::Sender は !Sync なので、実際には Mutex<Sender<...>> で保護

    // サブスクリプション dispatch 表
    subs: Mutex<CSRawMutex, HVec<&'static dyn SubscriptionSlot, MAX_SUBS>>,

    // 切断フラグ — 一度立ったらすべての send が Error::Disconnected を返す
    disconnected: AtomicBool,
}
```

### 5.2 送信パス (publish)

`embassy_sync::zerocopy_channel` を使った **コピー1回** の送信:

```rust
// 公開 API (generic — 薄い shim)
impl<M: Message> Publisher<M> {
    pub async fn publish(&self, msg: &M) -> Result<(), Error> {
        if self.ctx.is_disconnected() { return Err(Error::Disconnected); }

        // borrow next slot in zerocopy queue
        let slot = self.ctx.tx_send_lock().await;       // Mutex<Sender>
        let frame = slot.send().await;                  // &mut Frame

        // serialize CDR body in-place into frame.bytes
        let body_len = serialize_body_into(&mut frame.body, msg);

        // 非 generic helper — ROM 節約のためここで monomorphism を断ち切る
        finalize_write_data(frame, self.dw_id, self.ctx, body_len);

        slot.send_done();
        Ok(())
    }
}

// 非 generic の本体 (一度だけコンパイルされる)
fn finalize_write_data(frame: &mut Frame, dw_id: u16, ctx: &SessionInner, body_len: usize) {
    let seq = ctx.next_seq();
    finalize_write_data_headers(&mut frame.bytes[..total_len], session_id, seq, &key, dw_id);
}
```

`Frame` 構造:

```rust
struct Frame {
    bytes: [u8; FRAME_BUF_SIZE],   // ~384 bytes
    len:   usize,
}
```

queue 深さ `N = 2` で `Frame` × 2 = ~768 bytes 静的確保。

> **キーポイント**: `Publisher<M>` の generic は `serialize_body_into` の 1 関数だけ依存する。残り (キュー操作・WRITE_DATA ヘッダ・TCP 書き込み) は完全に非 generic。これにより `Publisher<Float32>` と `Publisher<Imu>` が共有するコードが最大化し、ROM が節約される。

### 5.3 受信パス (subscription)

既存の `Subscription<M, N>` / `SubscriptionSlot` をそのまま使う。`Executor` が dispatch:

```rust
impl Executor {
    async fn run(mut self) -> ! {
        loop {
            match select(self.tx_recv.receive(), read_one_frame(&mut self.socket, &mut self.rx_buf)).await {
                Either::First(frame) => {
                    let _ = framing::write_framed(&mut self.socket, &frame.bytes[..frame.len]).await;
                    self.tx_recv.receive_done();
                }
                Either::Second(Ok(len)) => {
                    self.dispatch(&self.rx_buf[..len]);
                }
                Either::Second(Err(_)) => {
                    self.ctx.set_disconnected();
                    self.creation_signal.signal(Err(Error::Disconnected));
                    // future: trigger reconnect
                    core::future::pending::<()>().await;
                }
            }
        }
    }

    fn dispatch(&self, msg: &[u8]) {
        // 既存の Session::dispatch_frame と同じ:
        //   SUBMSG_DATA  → subs slot の try_deliver
        //   SUBMSG_STATUS → creation_pending_req と一致なら creation_signal.signal
        //   それ以外     → debug! ログのみ
    }
}
```

### 5.4 CREATE 系 (entity 生成)

`Node::create_publisher` などは **executor タスクを経由しない** ことに注意 — 直接 tx_queue に push して、receive_done 後に creation_signal を待つ。

```rust
async fn send_create_and_wait(ctx: &SessionInner, build: impl FnOnce(&mut Frame, &SessionInner, u16) -> usize) -> Result<(), Error> {
    let _guard = ctx.creation_lock.lock().await;     // 1 件ずつ直列化

    let req_id = ctx.next_req();
    ctx.creation_signal.reset();
    ctx.creation_pending_req.store(req_id, Ordering::Release);

    {
        let mut slot = ctx.tx_send_lock().await;
        let frame = slot.send().await;
        let n = build(frame, ctx, req_id);
        frame.len = n;
        slot.send_done();
    }

    // executor が STATUS を見つけたら signal してくる
    let result = with_timeout(CREATE_TIMEOUT, ctx.creation_signal.wait()).await
        .map_err(|_| Error::Timeout)??;

    ctx.creation_pending_req.store(0, Ordering::Release);
    Ok(result)
}
```

`MAX_SUBS = 8`、`CREATE_TIMEOUT = 5s` あたりが妥当。

### 5.5 Mutex 粒度・ロック順序

- `creation_lock` (Outer): CREATE 系を直列化。
- `tx_send_lock` (Inner): tx queue への push を直列化。`creation_lock` 保持中に取って良い (ロック順守)。
- `subs` lock (独立): create_subscription 時の table 書き込み + executor dispatch 時の読み出しのみ。

publishe path は **`tx_send_lock` のみ** を取る → user task のクリティカルセクションは「serialize → push slot」だけで完結し、CREATE と競合しない。

### 5.6 メモリ・ROM 概算

| 項目                                 | RAM (bytes) | 備考                                      |
| ------------------------------------ | ----------- | ----------------------------------------- |
| `SessionInner` (atomic counters)     | ~32         |                                           |
| `creation_signal` + flag             | ~16         |                                           |
| `Mutex<Sender>`                      | ~16         |                                           |
| `tx_queue` storage (`Frame × 2`)     | ~800        | 384 byte body + headers slack             |
| `rx_buf` (executor)                  | 768         | 既存の `RX_BUF_SIZE`                      |
| `subs` HVec (8 × `&dyn`)             | ~136        | 各 16 bytes (ptr + vtable)                |
| `Subscription<M, N>` slots (per topic) | M ≤ 8 × ~64 + inbox | ユーザー宣言                  |
| **合計 (runtime)**                   | **~1.8 KB** | + per-subscription inbox                  |

ROM 増分 (現状の `Session` ベースから): 推定 +2 KB 程度 (zerocopy_channel + Signal mailbox)。`Publisher<M>::publish` を非 generic にする節約と相殺し、1 例当たりの増減はほぼゼロを目標。

### 5.7 エラー型

```rust
pub enum Error {
    Io,
    Disconnected,
    Timeout,                  // CREATE_TIMEOUT 越え
    BufferTooSmall,
    AgentRejected(u8),
    UnexpectedReply,
    StatusReqMismatch,
    TooManySubscriptions,
    SubscriptionOverflow,
    NotStarted,               // Runtime::start 前に Context が触られた
}
```

## 6. ファイル構成 (`external/micro_xrce_dds_rs/src/`)

```
lib.rs              # re-exports + client_key! macro (現状維持)
error.rs            # Error enum を Disconnected/Timeout/NotStarted で拡張
framing.rs          # 変更なし
protocol.rs         # 変更なし (定数 + low-level encoders は session.rs から移管)
cdr.rs / cdr_reader.rs # 変更なし
message.rs          # 変更なし
ros2/               # 変更なし

rt/                 # NEW — runtime layer
  mod.rs            # 公開: Runtime, Context, RuntimeConfig
  inner.rs          # SessionInner 定義
  executor.rs       # Executor::run (select ループ)
  encode.rs         # Session::* の wire encoders を非 generic 関数として整理 (= 現 session.rs の encode_*)
  creation.rs       # send_create_and_wait helper

node.rs             # Node::create_publisher / create_subscription / (将来) create_service
publisher.rs        # Publisher<M>::publish — generic は最薄, 内部は rt::encode を呼ぶ
subscription.rs     # 既存 + subscription_slot!() macro 追加

service.rs          # (v0.3) Service trait, ServiceServer<S>, ServiceClient<S>
action.rs           # (v0.4) Action trait, ActionServer<A>, ActionClient<A>
```

> 旧 `session.rs` は **削除** (中身は `rt/` と `node.rs` / `publisher.rs` / `subscription.rs` に分割移管)。
> ただし破壊的変更なので、移行は v0.2 リリースとして 1 step で行う (中間互換層は持たない)。

## 7. 将来拡張: Service / Action

### 7.1 Service (v0.3)

XRCE-DDS は `OBJK_REQUESTER` / `OBJK_REPLIER` をサポート。エンティティとしては:

- **Replier (server 側)**: 1 個の DataReader (request 受信) + 1 個の DataWriter (response 送信)
- **Requester (client 側)**: 1 個の DataWriter (request 送信) + 1 個の DataReader (response 受信)

ROS2 Service の DDS 表現は `rq/<service_name>Request` / `rr/<service_name>Reply` の 2 つの topic にマップされる。`SampleIdentity` (writer GUID + sequence) で request/response を対応付ける。

```rust
pub trait Service {
    type Request:  Message;
    type Response: Message;
    const TYPE_NAME: &'static str;       // e.g. "example_interfaces::srv::dds_::AddTwoInts_"
}

pub struct ServiceServer<S: Service> {
    requests:    &'static ServiceRequestSlot<S>,  // Subscription 風の inbox
    response_dw: u16,
    ctx:         Context,
}

impl<S: Service> ServiceServer<S> {
    pub async fn recv_request(&self) -> ServiceRequest<S>;  // SampleIdentity 内蔵
}

pub struct ServiceRequest<S: Service> {
    pub id:      SampleIdentity,
    pub payload: S::Request,
}

impl<S: Service> ServiceRequest<S> {
    pub async fn reply(self, ctx: Context, resp: &S::Response) -> Result<(), Error>;
}

pub struct ServiceClient<S: Service> { ... }
impl<S: Service> ServiceClient<S> {
    pub async fn call(&self, req: &S::Request) -> Result<S::Response, Error>;
    pub async fn call_with_timeout(&self, req: &S::Request, t: Duration) -> Result<S::Response, Error>;
}
```

実装上の追加要素:
- `rt/inner.rs` に **request/response mailbox** (`pending_call: Map<SequenceNumber, Signal<Response>>` 風) を追加。
- 受信 dispatch に「reply 種別なら mailbox lookup」を追加。
- Subscription dispatch table と service-call mailbox を統合した `Dispatcher` を切り出す。

### 7.2 Action (v0.4)

ROS2 Action = 3 service (`send_goal` / `get_result` / `cancel_goal`) + 2 topic (`feedback` / `status`)。Service / Subscription / Publisher が揃った後、それらを束ねるラッパとして実装:

```rust
pub trait Action {
    type Goal:     Message;
    type Result:   Message;
    type Feedback: Message;
}

pub struct ActionServer<A: Action> {
    send_goal:   ServiceServer<SendGoal<A>>,
    get_result:  ServiceServer<GetResult<A>>,
    cancel_goal: ServiceServer<CancelGoal>,
    feedback_pub: Publisher<FeedbackMsg<A>>,
    status_pub:   Publisher<GoalStatusArray>,
}

impl<A: Action> ActionServer<A> {
    pub async fn accept_goal(&self) -> GoalHandle<A>;
}

impl<A: Action> GoalHandle<A> {
    pub async fn publish_feedback(&self, fb: &A::Feedback) -> Result<(), Error>;
    pub async fn succeed(self, result: &A::Result) -> Result<(), Error>;
    pub async fn cancel(self) -> Result<(), Error>;
}
```

これは純粋な公開 API レイヤーで、wire 上の追加は無い (service と topic の組み合わせ)。

## 8. 移行戦略 (Breaking Changes)

v0.1 → v0.2 は破壊的変更。以下の例を全部書き換える:

- `examples/microros_hello.rs` — もっとも単純。1 task = 1 node。
- `examples/microros_subscriber.rs` — subscribe demo。task が `slot.recv().await` で完結。
- `examples/wifi_microros_sensors.rs` — 6 つの publisher を 3〜6 タスクに分割可能。`static Channel<f32, 4> × 6` を全廃。

旧 API の `Session::connect / spin*` 系は削除 (互換 shim は持たない)。

## 9. テスト方針

- **wire-format regress**: `tests/` に CREATE_CLIENT / CREATE_PARTICIPANT / WRITE_DATA / READ_DATA の生成バイト列の固定 fixture (現状の動作 baseline) を置き、リファクタ後に同一バイト列を生成することを確認。
- **dispatcher unit test**: 偽の `dyn SubscriptionSlot` を 3 つ登録して、合成した DATA フレームを `dispatch_data_payload` に流し、正しい slot に届くか確認。
- **on-target smoke test**: `microros_hello` (publish only) → `microros_subscriber` (subscribe only) → `wifi_microros_sensors` (mixed) を順に flash & `ros2 topic echo` 確認。
