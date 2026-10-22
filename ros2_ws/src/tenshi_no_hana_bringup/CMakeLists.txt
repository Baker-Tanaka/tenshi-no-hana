cmake_minimum_required(VERSION 3.8)
project(tenshi_no_hana_bringup)

find_package(ament_cmake REQUIRED)

install(DIRECTORY launch rviz urdf
  DESTINATION share/${PROJECT_NAME}
)

install(PROGRAMS scripts/imu_dead_reckoning.py
  DESTINATION lib/${PROJECT_NAME}
)

if(BUILD_TESTING)
  find_package(ament_cmake_pytest REQUIRED)
  ament_add_pytest_test(test_bringup_contract test/test_bringup_contract.py)
endif()

ament_package()
