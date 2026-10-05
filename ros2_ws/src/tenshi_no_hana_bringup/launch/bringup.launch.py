from pathlib import Path

from ament_index_python.packages import get_package_share_directory
from launch import LaunchDescription
from launch.actions import DeclareLaunchArgument
from launch.conditions import IfCondition
from launch.substitutions import Command, LaunchConfiguration
from launch_ros.actions import Node
from launch_ros.parameter_descriptions import ParameterValue


NAV2_NODES = (
    ("nav2_controller", "controller_server", "controller_server"),
    ("nav2_smoother", "smoother_server", "smoother_server"),
    ("nav2_planner", "planner_server", "planner_server"),
    ("nav2_behaviors", "behavior_server", "behavior_server"),
    ("nav2_bt_navigator", "bt_navigator", "bt_navigator"),
    ("nav2_waypoint_follower", "waypoint_follower", "waypoint_follower"),
    ("nav2_velocity_smoother", "velocity_smoother", "velocity_smoother"),
)


def generate_launch_description():
    package_share = Path(get_package_share_directory("tenshi_no_hana_bringup"))
    nav2_params = package_share / "config" / "nav2_odom_params.yaml"
    xacro_file = package_share / "urdf" / "tenshi_no_hana.urdf.xacro"
    rviz_config = package_share / "rviz" / "tenshi_no_hana.rviz"
    use_sim_time = LaunchConfiguration("use_sim_time")
    common_parameters = [str(nav2_params), {"use_sim_time": use_sim_time}]
    tf_remappings = [("/tf", "tf"), ("/tf_static", "tf_static")]

    nav2_actions = []
    for package, executable, name in NAV2_NODES:
        remappings = list(tf_remappings)
        if name in ("controller_server", "behavior_server"):
            remappings.append(("cmd_vel", "cmd_vel_nav"))
        elif name == "velocity_smoother":
            remappings.extend([
                ("cmd_vel", "cmd_vel_nav"),
                ("cmd_vel_smoothed", "/cmd_vel"),
            ])
        nav2_actions.append(Node(
            package=package,
            executable=executable,
            name=name,
            parameters=common_parameters,
            remappings=remappings,
            output="screen",
        ))

    nav2_actions.append(Node(
        package="nav2_lifecycle_manager",
        executable="lifecycle_manager",
        name="lifecycle_manager_navigation",
        parameters=[{
            "autostart": True,
            "node_names": [name for _, _, name in NAV2_NODES],
            "use_sim_time": use_sim_time,
        }],
        output="screen",
    ))

    return LaunchDescription([
        DeclareLaunchArgument("use_rviz", default_value="true"),
        DeclareLaunchArgument("use_sim_time", default_value="false"),
        Node(
            package="robot_state_publisher",
            executable="robot_state_publisher",
            parameters=[{
                "robot_description": ParameterValue(Command(["xacro ", str(xacro_file)]), value_type=str),
                "use_sim_time": use_sim_time,
            }],
        ),
        *nav2_actions,
        Node(
            package="rviz2",
            executable="rviz2",
            arguments=["-d", str(rviz_config)],
            condition=IfCondition(LaunchConfiguration("use_rviz")),
            output="screen",
        ),
    ])
