from pathlib import Path

from ament_index_python.packages import get_package_share_directory
from launch import LaunchDescription
from launch.actions import DeclareLaunchArgument, IncludeLaunchDescription
from launch.conditions import IfCondition
from launch.launch_description_sources import PythonLaunchDescriptionSource
from launch.substitutions import Command, LaunchConfiguration
from launch_ros.actions import Node
from launch_ros.parameter_descriptions import ParameterValue


def generate_launch_description():
    package_share = Path(get_package_share_directory("tenshi_no_hana_bringup"))
    nav2_share = Path(get_package_share_directory("nav2_bringup"))
    nav2_params = package_share / "config" / "nav2_odom_params.yaml"
    xacro_file = package_share / "urdf" / "tenshi_no_hana.urdf.xacro"
    rviz_config = package_share / "rviz" / "tenshi_no_hana.rviz"

    return LaunchDescription([
        DeclareLaunchArgument("use_rviz", default_value="true"),
        DeclareLaunchArgument("use_sim_time", default_value="false"),
        Node(
            package="robot_state_publisher",
            executable="robot_state_publisher",
            parameters=[{
                "robot_description": ParameterValue(Command(["xacro ", str(xacro_file)]), value_type=str),
                "use_sim_time": LaunchConfiguration("use_sim_time"),
            }],
        ),
        IncludeLaunchDescription(
            PythonLaunchDescriptionSource(str(nav2_share / "launch" / "navigation_launch.py")),
            launch_arguments={
                "params_file": str(nav2_params),
                "use_sim_time": LaunchConfiguration("use_sim_time"),
            }.items(),
        ),
        Node(
            package="rviz2",
            executable="rviz2",
            arguments=["-d", str(rviz_config)],
            condition=IfCondition(LaunchConfiguration("use_rviz")),
            output="screen",
        ),
    ])
