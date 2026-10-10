# Angel's Nose - ROS2 × Baker link. Dev × Rover

> ⚠️ This project is under active development. Features, wiring, and documentation may change.

**A 2WD rover remotely controlled from ROS 2/Nav2.**

## Illustrative Image

![Project preview](docs/img/image.png)

![System diagram](docs/img/tenshi_no_hana_rover_v2.svg)

> 日本語版はこちら: [README.ja.md](./README.ja.md)

## Overview
Angel's Nose is a compact 2-wheel rover based on the Baker link. Dev Rev.1 (RP2040) board. It patrols distilleries and barrel aging warehouses, detects ethanol vapor, and estimates barrel liquid surface using magnetic sensing.

This project combines:
- **micro_xrce_dds_rs** with a **micro-ROS Agent** bridge
- **XIAO-ESP32-C3 as a Wi-Fi ESP-hosted MCU**
- **Embassy-rs no_std async runtime**
- **micro_xrce_dds_rs** with a **micro-ROS Agent** bridge

## Key features
- Ethanol vapor detection as a proxy for the barrel's "angel's share"
- Non-contact barrel liquid level sensing using a floating magnet and magnetic field sensor
- 6-axis IMU (ICM-20602) for odometry and attitude estimation
- Wi-Fi communication via XIAO-ESP32-C3
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

- **MQ-3B AOUT → GP26 (ADC0)** — AOUT swings up to ~5 V. Allow ~2 minutes warm-up after power-on; the firmware suppresses publishing during this period.
- **HC-SR04 Echo → GP3** — Echo outputs 5 V pulses.

### Motor drive

The kit's gear motors are rated **3 V** while DRV8835 VM is fed from the 5 V rail.
Limit the PWM duty to **~60 %** in firmware to keep the effective motor voltage near 3 V
(DRV8835 max 1.5 A/ch; place a bulk capacitor near VM).

## Wireless stack
- Uses **XIAO-ESP32-C3** with ESP-hosted MCU firmware
- Uses the `external/embassy` submodule and `embassy-net-esp-hosted` from the `upstream-esp-hosted-mcu` fork

## Software stack
- `micro_xrce_dds_rs` with micro-ROS Agent TCP transport
- Custom message support for `angel_nose_msgs` (ethanol level, liquid height, environment data)

## Firmware architecture

The firmware uses a static launch composition rather than a runtime ROS 2
launch file. [`src/main.rs`](src/main.rs) is only the Embassy entry point:
it initializes the RP2040, assembles board resources, and calls
[`src/launch.rs`](src/launch.rs). To add or remove a node, change the spawn list
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
in its own folder. The launch composition moves each hardware resource into
the one task that uses it, while passing an `embassy_ros::Context` and typed
endpoints to nodes. No node selects a local versus network publisher.
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
remain application-specific. A transport failure is logged; this composition
does not implement automatic reconnection.

The optional `embassy_ros::transport::embassy_net::Agent` adapter owns the TCP
connection/handshake attempt. The application supplies the endpoint, static
`TcpBuffers` and retry policy; a failed attempt returns the socket for reuse.
Both connection and handshake failures are logged and retried during startup.
Board-specific Wi-Fi setup remains in `network.rs`.

ROS `.msg` generation is provided by the host-only companion
[`micro-xrce-dds-codegen`](external/micro_xrce_dds_rs/codegen/README.md).
The application build script retains Wi-Fi configuration and selects message
inputs and fixed capacities. Generated Header types offer `set_stamp`, and
the low-level backend retains queue-admission deadlines. Framework publishing
is nonblocking keep-last acceptance, not delivery ACKs or reliable event delivery.

`launch.rs` initializes the framework once and prepares all endpoints without
network I/O. It selects ROS directions with `context.export` / `context.import`,
then spawns sensor, motor, odometry and network tasks. The framework connection
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
- Wheel encoder or external localization integration for higher-accuracy Nav2
- Autonomous mapping and multi-rover operation

## ESP32-Hosted (ESP32-C3) pin mapping
Pin assignments are extracted from `sdkconfig`.

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
