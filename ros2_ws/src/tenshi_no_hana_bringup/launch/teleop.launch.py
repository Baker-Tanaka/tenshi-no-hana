from pathlib import Path

from ament_index_python.packages import get_package_share_directory
from launch import LaunchDescription
from launch.actions import DeclareLaunchArgument
from launch.conditions import IfCondition
from launch.substitutions import Command, LaunchConfiguration
from launch_ros.actions import Node
from launch_ros.parameter_descriptions import ParameterValue


def generate_launch_description():
    package_share = Path(get_package_share_directory("tenshi_no_hana_bringup"))
    xacro_file = package_share / "urdf" / "tenshi_no_hana.urdf.xacro"
    rviz_config = package_share / "rviz" / "teleop.rviz"
    return LaunchDescription([
        DeclareLaunchArgument("use_rviz", default_value="true"),
        Node(
            package="robot_state_publisher",
            executable="robot_state_publisher",
            parameters=[{
                "robot_description": ParameterValue(
                    Command(["xacro ", str(xacro_file)]), value_type=str
                ),
                "use_sim_time": False,
            }],
            output="screen",
        ),
        Node(
            package="rviz2",
            executable="rviz2",
            arguments=["-d", str(rviz_config)],
            condition=IfCondition(LaunchConfiguration("use_rviz")),
            output="screen",
        ),
    ])
