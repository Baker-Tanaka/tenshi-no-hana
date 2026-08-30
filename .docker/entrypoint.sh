#!/bin/bash
set -e

# Source ROS2 setup
source /opt/ros/jazzy/setup.bash
source /opt/micro_ros_agent_ws/install/setup.bash
if [[ -f /tenshi-no-hana/ros2_ws/install/setup.bash ]]; then
	source /tenshi-no-hana/ros2_ws/install/setup.bash
fi

if [[ "${MICROROS_AGENT_AUTOSTART:-0}" == "1" ]]; then
	ros2 run micro_ros_agent micro_ros_agent tcp4 --port "${MICROROS_AGENT_PORT:-8888}" -v "${MICROROS_AGENT_VERBOSITY:-5}" \
		> /tmp/micro-ros-agent.log 2>&1 &
	echo "micro-ROS Agent started on TCP port ${MICROROS_AGENT_PORT:-8888}"
fi

exec "$@"
