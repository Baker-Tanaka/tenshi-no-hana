cmake_minimum_required(VERSION 3.16)
project(std_msgs)
find_package(ament_cmake REQUIRED)
find_package(rosidl_cmake REQUIRED)
find_package(rosidl_generator_microxrcedds REQUIRED)
find_package(builtin_interfaces REQUIRED)
rosidl_generate_interfaces(${PROJECT_NAME}
  "msg/Float32.msg" "msg/Header.msg" DEPENDENCIES builtin_interfaces)
ament_package()
