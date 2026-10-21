cmake_minimum_required(VERSION 3.16)
project(tf2_msgs)
find_package(ament_cmake REQUIRED)
find_package(rosidl_cmake REQUIRED)
find_package(rosidl_generator_microxrcedds REQUIRED)
find_package(geometry_msgs REQUIRED)
rosidl_generate_interfaces(${PROJECT_NAME}
  "msg/TFMessage.msg" DEPENDENCIES geometry_msgs)
ament_package()
