#!/bin/bash
set -e

# Source ROS2 setup
source /opt/ros/jazzy/setup.bash

# Set rmw implementation
export RMW_IMPLEMENTATION=rmw_zenoh_cpp

# Configure zenoh to connect to the router service
export ZENOH_ROUTER_CHECK_ATTEMPTS=5

exec "$@"
