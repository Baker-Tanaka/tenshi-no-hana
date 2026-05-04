#!/bin/bash
set -e

# Source ROS2 setup
source /opt/ros/jazzy/setup.bash

exec "$@"
