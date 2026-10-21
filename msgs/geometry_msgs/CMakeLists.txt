cmake_minimum_required(VERSION 3.16)
project(geometry_msgs)
find_package(ament_cmake REQUIRED)
find_package(rosidl_cmake REQUIRED)
find_package(rosidl_generator_microxrcedds REQUIRED)
find_package(std_msgs REQUIRED)
rosidl_generate_interfaces(${PROJECT_NAME}
  "msg/Point.msg" "msg/Pose.msg" "msg/PoseWithCovariance.msg" "msg/Quaternion.msg"
  "msg/Transform.msg" "msg/TransformStamped.msg" "msg/Twist.msg"
  "msg/TwistWithCovariance.msg" "msg/Vector3.msg"
  DEPENDENCIES std_msgs)
ament_package()
