// ros_msgs.rs — auto-generated ROS2 message types from `msgs/`.
//
// This file is just a `include!` shim: build.rs parses every `.msg` under
// `msgs/{package}/msg/*.msg`, walks the AST returned by `roslibrust_codegen`,
// and emits a no_std module tree under `$OUT_DIR/ros_msgs.rs`. We pull it in
// here so it can be referenced via `#[path = "../src/ros_msgs.rs"]` from any
// example.
//
// To regenerate: edit any `.msg` (or build.rs itself) — cargo's
// `rerun-if-changed` will re-run the build script.
//
// Usage:
//   #[path = "../src/ros_msgs.rs"]
//   mod ros_msgs;
//   use ros_msgs::nav_msgs::Odometry;
//   use ros_msgs::tf2_msgs::TFMessage;
//   use ros_msgs::geometry_msgs::TransformStamped;

#![allow(dead_code, non_snake_case, unused_variables, clippy::all)]

include!(concat!(env!("OUT_DIR"), "/ros_msgs.rs"));
