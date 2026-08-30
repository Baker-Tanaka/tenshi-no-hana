from pathlib import Path

from ament_index_python.packages import get_package_share_directory
from launch import LaunchDescription
from launch.actions import DeclareLaunchArgument, IncludeLaunchDescription, OpaqueFunction
from launch.conditions import IfCondition
from launch.launch_description_sources import PythonLaunchDescriptionSource
from launch.substitutions import Command, LaunchConfiguration
from launch_ros.actions import Node, SetRemap
from launch_ros.parameter_descriptions import ParameterValue


def _nav2_actions(context):
    package_share = Path(get_package_share_directory("tenshi_no_hana_bringup"))
    nav2_share = Path(get_package_share_directory("nav2_bringup"))
    mode = LaunchConfiguration("localization_mode").perform(context)
    map_path = LaunchConfiguration("map").perform(context)

    if mode not in ("odom", "amcl"):
        raise RuntimeError("localization_mode must be 'odom' or 'amcl'")
    if mode == "amcl" and not map_path:
        raise RuntimeError("map must be provided when localization_mode:=amcl")

    params_name = "nav2_odom_params.yaml" if mode == "odom" else "nav2_amcl_params.yaml"
    params_file = str(package_share / "config" / params_name)
    actions = []

    if mode == "amcl":
        actions.append(SetRemap(src="/scan", dst=LaunchConfiguration("scan_topic")))
        actions.append(
            IncludeLaunchDescription(
                PythonLaunchDescriptionSource(str(nav2_share / "launch" / "localization_launch.py")),
                launch_arguments={
                    "map": map_path,
                    "params_file": params_file,
                    "use_sim_time": LaunchConfiguration("use_sim_time"),
                }.items(),
            )
        )

    actions.append(
        IncludeLaunchDescription(
            PythonLaunchDescriptionSource(str(nav2_share / "launch" / "navigation_launch.py")),
            launch_arguments={
                "params_file": params_file,
                "use_sim_time": LaunchConfiguration("use_sim_time"),
            }.items(),
        )
    )
    return actions


def generate_launch_description():
    package_share = Path(get_package_share_directory("tenshi_no_hana_bringup"))
    xacro_file = package_share / "urdf" / "tenshi_no_hana.urdf.xacro"
    rviz_config = package_share / "rviz" / "tenshi_no_hana.rviz"

    return LaunchDescription([
        DeclareLaunchArgument("localization_mode", default_value="odom"),
        DeclareLaunchArgument("map", default_value=""),
        DeclareLaunchArgument("scan_topic", default_value="/scan"),
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
        OpaqueFunction(function=_nav2_actions),
        Node(
            package="rviz2",
            executable="rviz2",
            arguments=["-d", str(rviz_config)],
            condition=IfCondition(LaunchConfiguration("use_rviz")),
            output="screen",
        ),
    ])
