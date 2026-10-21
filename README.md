# Angel's Nose - ROS2 × Baker link. Dev × Rover

> ⚠️ This project is under active development. Features, wiring, and documentation may change.

**A 2WD rover remotely controlled from ROS 2/Nav2.**

## Illustrative Image

![Project preview](docs/img/image.png)

![System diagram](docs/img/tenshi_no_hana_rover_v2.svg)

> 日本語版はこちら: [README.ja.md](./README.ja.md)

## Overview
Angel's Nose is a compact 2-wheel rover based on the Baker link. Dev Rev.1 (RP2040) board. The current system targets ROS 2-controlled motion, IMU and ultrasonic sensing, and open-loop odometry. Distillery patrol, ethanol sensing and magnetic liquid-level estimation are future extensions, not implemented features.

This project combines:
- **ESP32-C6 as a Wi-Fi ESP-hosted MCU (slave firmware 2.12.9)**
- **Embassy-rs no_std async runtime**
- **micro_xrce_dds_rs** with a **micro-ROS Agent** bridge

## Key features
- ROS 2 velocity commands with a 500 ms motor watchdog
- 6-axis IMU (ICM-20602) acceleration and angular velocity (no attitude fusion)
- HC-SR04 ranging and open-loop wheel-command odometry
- Wi-Fi communication via ESP32-C6
- Low-cost and lightweight no_std embedded design

## Hardware bill of materials
Estimated Japan-focused pricing as of April 2026.

| Part              | Example                                           | Approx. Price | Link                                                            | Notes                                                         |
| ----------------- | ------------------------------------------------- | ------------- | --------------------------------------------------------------- | ------------------------------------------------------------- |
| 2WD robot chassis | 2WD Mini Smart Robot Mobile Platform Kit          | ¥1,900        | [Akizuki](https://akizukidenshi.com/catalog/g/g113651/)         | Includes two FA-130 class 3 V gear motors                     |
| Baker link. Dev   | Rev. 1                                            | ¥1,980        | [Switch Science](https://www.switch-science.com/products/10044) | Main MCU; native Embassy-rs / Rust no_std support             |
| Motor driver      | AE-DRV8835-S (DRV8835 stepping & DC motor driver) | ¥400          | [Akizuki](https://akizukidenshi.com/catalog/g/g109848/)         | PH/EN mode (MODE pin tied to VCC)                             |
| IMU               | ICM-20602 Breakout Board                          | ¥990          | [Switch Science](https://www.switch-science.com/products/8695)  | 6-axis (accel + gyro), I2C addr 0x68                          |
| Ethanol sensor    | MQ-3B or MQ-3 module                              | ¥450          | [Akizuki](https://akizukidenshi.com/catalog/g/g116269/)         | Main sensor for vapor detection; heater requires 5 V / 150 mA |
| Ultrasonic sensor | HC-SR04                                           | ¥300          | [Switch Science](https://www.switch-science.com/products/8224/) | For obstacle avoidance; 5 V device                            |
| 3.3 V regulator   | NJU7223F33 (3.3 V / 500 mA LDO)                   | ¥80           | [Akizuki](https://akizukidenshi.com/catalog/g/g100432/)         | 9 V → 3.3 V rail (logic)                                      |
| 5 V regulator     | BP5293-50 (5 V / 1 A DC-DC)                       | ¥240          | [Akizuki](https://akizukidenshi.com/catalog/g/g111188/)         | 9 V → 5 V rail (motors, MQ-3B heater, HC-SR04)                |
| Battery           | 9 V (006P) + battery snap                         | ~¥500         | —                                                               | Feeds both regulators; add a power switch                     |
| Voltage dividers  | 10 kΩ + 15 kΩ resistors ×2                        | < ¥20         | —                                                               | Required: scale MQ-3B AOUT and HC-SR04 Echo (5 V) to 3.0 V    |

## Wiring

![](docs/schematics/tenshi_no_hana_wiring.svg)

> The illustrations and C3 tables below describe the historical XIAO ESP32-C3 design.
> The current hardware is ESP32-C6, running ESP-Hosted MCU slave 2.12.9.
> Use the [verified current GPIO mapping](docs/DESIGN.md#ピンと電源), not the C3 tables.

### Power supply

```
9 V battery (006P)
  ├── BP5293-50  → 5 V rail   : XIAO 5V pin, MQ-3B VCC (heater), HC-SR04 VCC, DRV8835 VM
  └── NJU7223F33 → 3.3 V rail : Baker link. Dev 3V3, ICM-20602 VCC, DRV8835 VCC (logic)
```

> - **Disconnect the battery while USB is connected** for flashing/debugging.
> - BP5293-50 needs **≥ 8 V input**; watch the 9 V battery level.
> - NJU7223F33 dissipates (9 V − 3.3 V) × load as heat; keep the 3.3 V load light and provide copper area for cooling.

### Signal-level notes (5 V → 3.3 V)

The RP2040 GPIOs are **not 5 V tolerant**. Two signals need a **10 kΩ + 15 kΩ voltage divider** (5 V → 3.0 V):

- **MQ-3B AOUT → GP26 (ADC0)** — Reserved for a future sensor node; ADC acquisition and warm-up handling are not implemented. AOUT can reach ~5 V.
- **HC-SR04 Echo → GP3** — Echo outputs 5 V pulses.

### Motor drive

The kit's gear motors are rated **3 V** while DRV8835 VM is fed from the 5 V rail.
Limit the PWM duty to **~60 %** in firmware to keep the effective motor voltage near 3 V
(DRV8835 max 1.5 A/ch; place a bulk capacitor near VM).

## Wireless stack
- Uses **ESP32-C6** with ESP-Hosted MCU slave firmware **2.12.9**
- Uses the `external/embassy` submodule and `embassy-net-esp-hosted` from the `upstream-esp-hosted-mcu` fork

## Software stack
- `micro_xrce_dds_rs` with micro-ROS Agent TCP transport
- Generated `no_std` standard ROS message crates; no C RMW library is linked into the firmware

## Firmware build

```sh
git submodule update --init --recursive
cp wifi_config.json.example wifi_config.json
# Edit wifi_config.json locally: SSID, password, Agent IPv4 address and port.
cargo build --locked
cargo build --release --locked
```

The toolchain is pinned in [rust-toolchain.toml](rust-toolchain.toml).
Install `flip-link` (the development container already provides it).
Do not commit Wi-Fi credentials. Invalid JSON, SSID size or Agent endpoints
fail the build instead of producing a firmware with fallback settings.
The current ESP-hosted fork limits passwords to 32 UTF-8 bytes; longer
passwords are rejected at build time rather than retried indefinitely.
The default feature is `wifi`; the binary requires this feature.
Keep `opt-level = 2` for both application and dependencies in debug builds.
Dependency-only optimization does not bound the stack of protobuf decoders
monomorphized in the application. Debug symbols and assertions stay enabled.

Pure rover math can be tested without a board:

```sh
cargo test --lib --no-default-features --target x86_64-unknown-linux-gnu
```

After changing [message definitions](msgs/) or device capacities, regenerate
inside the ROS Jazzy development container:

```sh
bash external/micro_xrce_dds_rs/codegen/regenerate.sh
# Check definitions, generator and generated output agree without rewriting:
bash external/micro_xrce_dds_rs/codegen/regenerate.sh --check
```

See the [generator documentation](external/micro_xrce_dds_rs/codegen/README.md)
for ament integration and tests. Commit regenerated crates together with their
inputs. Changes inside a submodule must be committed and published in that
repository before the parent can pin their revision; a parent-only commit
cannot preserve uncommitted submodule source changes.

## Development container

Open the repository in VS Code and run **Dev Containers: Rebuild and Reopen in
Container**. The [Dev Containers configuration](.devcontainer/devcontainer.json)
builds the [Dockerfile](.docker/Dockerfile) directly; Docker Compose is not used.

The repository is mounted at `/tenshi-no-hana` and commands run as `vscode`.
The image entrypoint starts the micro-ROS Agent on TCP port `8888`, published
on host port `8888` so the rover can connect to the host's LAN address.
`host.docker.internal` resolves to the host gateway inside the container.
Agent output is available at `/tmp/micro-ros-agent.log` inside the container.
Use **Dev Containers: Reopen in Container** to start it again after closing VS Code.
To restart the container and its Agent from the host, run:

```sh
docker restart tenshi-no-hana-container
```

## Firmware architecture

The firmware uses a static launch composition rather than a runtime ROS 2
launch file. [`src/main.rs`](src/main.rs) is only the Embassy entry point:
it initializes the RP2040 and ROS context, gives the spawner to `NodeSpawner`,
assembles board resources, and calls
[`src/launch.rs`](src/launch.rs). To add or remove a node, change the node startup list
in the launch module, not the entry point.

| Module | Responsibility |
| --- | --- |
| [`board.rs`](src/board.rs) | Pin assignments, interrupt bindings, construction of hardware resources |
| [`drivers/`](src/drivers/) | DRV8835, HC-SR04 and ICM-20602 device I/O; no ROS entities |
| [`network.rs`](src/network.rs) | ESP-hosted and network runner tasks, Wi-Fi/DHCP/TCP setup and XRCE driver startup |
| [`launch.rs`](src/launch.rs) | Application task composition and startup order |
| [`nodes/imu/`](src/nodes/imu/) | `/imu/data` |
| [`nodes/range/`](src/nodes/range/) | `/ultrasonic/front` |
| [`nodes/motor/`](src/nodes/motor/) | `/cmd_vel`, watchdog and coherent realized-motion snapshot |
| [`nodes/odometry/`](src/nodes/odometry/) | Open-loop `/odom` and `/tf` |

Each node keeps its task implementation, topics and node-specific configuration
in its own folder. Each node is one ordinary async function marked with
`#[embassy_ros::node(name = "...")]`, with publishers/subscriptions declared on
typed arguments. There is no setup/run tuple or separate startup function.
Launch lists `nodes.register(imu::node, board.imu)?` and equivalent registrations
(odometry receives `()`). The framework owns task allocation and spawning.
Topics and the generated task are private; endpoint creation and ROS connection
are automatic. Topic delivery is shared, but DDS endpoints belong to each node.
Launch transfers hardware ownership; it does not pass publishers/subscriptions.
No node selects a local versus network publisher.
The IMU task owns its I2C bus directly; it does not need a shared-bus cell.
Motor and odometry tasks share one short critical-section-protected snapshot,
so velocity components and their update time cannot come from different samples.

Sensing, motor control and odometry start before Wi-Fi/Agent connection.
The motor watches the framework's versioned connection status, stops on
connection loss, and rejects stale/expired commands. Its 500 ms watchdog uses
the command's original device-admission time, not the later dequeue time.
Remote `/cmd_vel` operation
still requires Wi-Fi and a micro-ROS Agent; offline mode continues local
sampling and state estimation, not autonomous driving.

The public framework is
[`embassy-ros`](external/micro_xrce_dds_rs/embassy_ros/README.md):
`embassy_ros::init()` -> `Context` -> `Node` -> typed `Topic` endpoints.
Static storage makes capacities explicit without exposing Embassy channels.
[`micro_xrce_dds_rs`](external/micro_xrce_dds_rs/) remains its low-level backend.
The framework owns entity creation, routing and clock synchronization.
Nodes read `context.clock().now()` at sampling time. Before the first
successful synchronization the clock
uses monotonic time since boot, as before; this is not Unix time.
Connection retry policy, sensor calibration, motor limits and the node graph
remain application-specific. Transport failures are logged and the application
retries the Agent connection. Wi-Fi link loss triggers reassociation; the
ESP-hosted heartbeat notifier is used instead of the driver's panic-on-timeout
default. Hardware initialization still depends on a responsive slave.

The optional `embassy_ros::transport::embassy_net::Agent` adapter owns the TCP
connection/handshake attempt. The application supplies the endpoint, static
`TcpBuffers` and retry policy; a failed attempt returns the socket for reuse.
Both connection and handshake failures are logged and retried during startup.
Board-specific Wi-Fi setup remains in `network.rs`.

ROS message generation uses the official ROS IDL adapter/parser through the
host-only [generator](external/micro_xrce_dds_rs/codegen/README.md), following
the generator/runtime separation in
[ros2_rust](https://github.com/ros2-rust/ros2_rust/blob/main/docs/message-generation.md).
The [generated package crates](generated/) are checked in and use `no_std`
types and CDR serialization, without C RMW bindings. Unbounded fields have
device-local capacities of 32-byte strings and one sequence element, matching
the single transform emitted on `/tf`. Regenerate when definitions or capacities
change; normal firmware builds do not run Python or require ROS.
The application build script only embeds validated Wi-Fi configuration and
sets the linker search path. Generated Header types offer `set_stamp`.
Framework publishing
is nonblocking keep-last acceptance, not delivery ACKs or reliable event delivery.

`main.rs` initializes the framework once. `launch.rs` registers the node
definitions, motor first. The framework synchronously prepares endpoints and
ROS directions before scheduling each task, without network I/O. Registration
failures include node/topic/phase and poison configuration, preventing later
registration or network startup. Main explicitly calls `nodes.finish()` after
registration and before starting networking. There is no global activation barrier
or rollback of previously spawned tasks. The framework connection
task bridges the existing ROS interfaces without node-specific bridge loops.
Network startup may continue
retrying indefinitely without blocking local control. Existing ROS topic names
and motor limits are unchanged.

This `launch.rs` is the embedded equivalent of choosing a ROS node graph; it
does not replace the host-side `ros2 launch` files, which configure host ROS 2
processes such as Nav2 and RViz.

## How it works
## ROS 2 interface

| Direction | Topic               | Type                  | Purpose                                     |
| --------- | ------------------- | --------------------- | ------------------------------------------- |
| Subscribe | `/cmd_vel`          | `geometry_msgs/Twist` | Remote velocity command                     |
| Publish   | `/imu/data`         | `sensor_msgs/Imu`     | ICM-20602 acceleration and angular velocity |
| Publish   | `/ultrasonic/front` | `sensor_msgs/Range`   | HC-SR04 front obstacle distance             |
| Publish   | `/odom`             | `nav_msgs/Odometry`   | Open-loop pose and realized velocity        |
| Publish   | `/tf`               | `tf2_msgs/TFMessage`  | `odom` to `base_link` transform             |

The firmware synchronizes message timestamps with the micro-ROS Agent. IMU
orientation is intentionally unavailable, and odometry is open-loop because
this chassis has no wheel encoders. Use external localization for accurate
Nav2 operation.

## RViz2 and Nav2

Rebuild the development container after changing `.docker/Dockerfile`, then
build and source the host package:

```sh
cd ros2_ws
colcon build --symlink-install
source install/setup.bash
```

Start the default odom-relative mode after the rover and micro-ROS Agent are
connected:

```sh
ros2 launch tenshi_no_hana_bringup bringup.launch.py
```

This mode supports short, low-speed relative goals without a map. In RViz2,
use **2D Goal Pose** to send a goal. Start with the wheels raised, then test
goals of 0.25-0.5 m on the floor. Cancelling Nav2 or stopping velocity output
causes the firmware watchdog to stop the motors within 500 ms.

The HC-SR04 only covers a narrow area in front of the rover. It improves
near-field obstacle detection but cannot replace a lidar for localization,
SLAM, or full-surround collision avoidance. The current odometry integrates
commanded motor speed without encoders and accumulates unbounded position
error. Measure and update the provisional dimensions in the
[robot description](ros2_ws/src/tenshi_no_hana_bringup/urdf/tenshi_no_hana.urdf.xacro)
before relying on the costmap footprint.
See [docs/DESIGN.md](docs/DESIGN.md) for the canonical hardware and system
design.

## Future work
- MQ-3B acquisition/calibration and magnetic liquid-level sensing
- Wheel encoder or external localization integration for higher-accuracy Nav2
- Autonomous mapping and multi-rover operation

## Historical ESP32-Hosted (ESP32-C3) pin mapping

This section is historical, not the current ESP32-C6 configuration.
See the [current GPIO mapping](docs/DESIGN.md#ピンと電源).
These are the intended pin assignments. The version and `sdkconfig` of the
previously working ESP-hosted slave are not recorded; confirm them against the
running device before rebuilding its firmware.

> Reference: [Seeed Studio XIAO ESP32C3 Getting Started](https://wiki.seeedstudio.com/XIAO_ESP32_C3_Getting_Started/)

### XIAO ESP32C3 GPIO ↔ physical pin mapping

| XIAO pin | GPIO   | Default function | Notes                         |
| -------- | ------ | ---------------- | ----------------------------- |
| D0       | GPIO2  | ADC              | ⚠️ Strapping pin               |
| D1       | GPIO3  | ADC              |                               |
| D2       | GPIO4  | ADC              | MTMS (JTAG)                   |
| D3       | GPIO5  | ADC              | MTDI (JTAG)                   |
| D4       | GPIO6  | SDA (I2C)        | FSPICLK, MTCK (JTAG)          |
| D5       | GPIO7  | SCL (I2C)        | FSPID, MTDO (JTAG)            |
| D6       | GPIO21 | UART TX          |                               |
| D7       | GPIO20 | UART RX          |                               |
| D8       | GPIO8  | SPI SCK          | ⚠️ Strapping pin               |
| D9       | GPIO9  | SPI MISO         | ⚠️ Strapping pin / BOOT button |
| D10      | GPIO10 | SPI MOSI         | FSPICS0                       |

### Recommended esp-hosted slave wiring

> Avoid all strapping pins (GPIO2/8/9) for stable operation.

| Signal     | GPIO   | XIAO pin | Purpose                   |
| ---------- | ------ | -------- | ------------------------- |
| SPI MOSI   | GPIO7  | D5       | Required                  |
| SPI MISO   | GPIO5  | D3       | Required                  |
| SPI CLK    | GPIO6  | D4       | Required                  |
| SPI CS     | GPIO10 | D10      | Required                  |
| Handshake  | GPIO3  | D1       | Timing sync               |
| Data Ready | GPIO4  | D2       | Data arrival notification |
| Reset      | GPIO21 | D6       | Recommended               |

#### Changes from previous wiring

| Signal   | Old GPIO   | New GPIO    | Reason                                   |
| -------- | ---------- | ----------- | ---------------------------------------- |
| SPI MISO | GPIO2 (D0) | GPIO5 (D3)  | Avoid strapping pin and boot instability |
| Reset    | none       | GPIO21 (D6) | Add host reset control                   |

#### Spare pins

| XIAO pin | GPIO   | Notes                            |
| -------- | ------ | -------------------------------- |
| D0       | GPIO2  | Spare, but avoid strapping pin   |
| D7       | GPIO20 | Spare UART RX for debugging      |
| D8       | GPIO8  | Spare, but avoid strapping pin   |
| D9       | GPIO9  | Spare, but avoid BOOT button pin |

### ESP-IDF menuconfig setup

```powershell
idf.py menuconfig
```

Navigate to **Example Configuration → Bus Config → SPI Full-Duplex Configuration** and set:

| Option            | Value |
| ----------------- | ----- |
| SPI MOSI (GPIO)   | 7     |
| SPI MISO (GPIO)   | 5     |
| SPI CLK (GPIO)    | 6     |
| SPI CS (GPIO)     | 10    |
| Handshake (GPIO)  | 3     |
| Data Ready (GPIO) | 4     |
| Reset pin (GPIO)  | 21    |

Save and rebuild.

```bash
# Create esp-hosted-mcu slave sample (ESP-IDF v5.3+ assumed)
idf.py create-project-from-example "espressif/esp_hosted:slave"
cd slave
idf.py set-target esp32c3
idf.py menuconfig
# Set SPI Full-Duplex Configuration:
#   MISO=5, MOSI=7, CLK=6, CS=10, HS=3, DR=4
#   Reset GPIO=21
idf.py build flash
```

## Submodule setup

RP2040-side dependencies are managed via git submodules:

```bash
git submodule update --init --recursive external/embassy external/micro_xrce_dds_rs
```
