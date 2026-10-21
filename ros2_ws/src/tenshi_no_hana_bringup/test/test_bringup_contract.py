from pathlib import Path
import xml.etree.ElementTree as ET

import yaml


PACKAGE_ROOT = Path(__file__).parents[1]


def load_yaml(name):
    with (PACKAGE_ROOT / "config" / name).open() as stream:
        return yaml.safe_load(stream)


def test_metadata_and_xacro_are_well_formed_xml():
    ET.parse(PACKAGE_ROOT / "package.xml")
    ET.parse(PACKAGE_ROOT / "urdf" / "tenshi_no_hana.urdf.xacro")


def test_odom_mode_matches_firmware_contract():
    params = load_yaml("nav2_odom_params.yaml")
    assert params["bt_navigator"]["ros__parameters"]["global_frame"] == "odom"
    assert params["bt_navigator"]["ros__parameters"]["robot_base_frame"] == "base_link"
    assert params["bt_navigator"]["ros__parameters"]["odom_topic"] == "/odom"
    local = params["local_costmap"]["local_costmap"]["ros__parameters"]
    assert local["range_layer"]["topics"] == ["/ultrasonic/front"]
    assert local["range_layer"]["plugin"] == "nav2_costmap_2d::RangeSensorLayer"
    global_costmap = params["global_costmap"]["global_costmap"]["ros__parameters"]
    assert global_costmap["range_layer"]["topics"] == ["/ultrasonic/front"]
    assert global_costmap["range_layer"]["plugin"] == "nav2_costmap_2d::RangeSensorLayer"


def test_launch_is_odom_only():
    launch_source = (PACKAGE_ROOT / "launch" / "bringup.launch.py").read_text()
    assert "nav2_odom_params.yaml" in launch_source
    assert 'DeclareLaunchArgument("use_rviz", default_value="true")' in launch_source
    assert 'package="rviz2"' in launch_source
    assert '("cmd_vel_smoothed", "/cmd_vel")' in launch_source
    for stale_name in (
        "amcl",
        "collision_monitor",
        "docking_server",
        "localization_mode",
        "route_server",
        "scan_topic",
    ):
        assert stale_name not in launch_source


def test_urdf_links_are_connected_to_base_link():
    robot = ET.parse(PACKAGE_ROOT / "urdf" / "tenshi_no_hana.urdf.xacro").getroot()
    links = {
        link.attrib["name"]
        for link in robot.findall("link")
        if "${" not in link.attrib["name"]
    }
    children_by_parent = {}
    for joint in robot.findall("joint"):
        parent = joint.find("parent").attrib["link"]
        child = joint.find("child").attrib["link"]
        children_by_parent.setdefault(parent, set()).add(child)

    xacro_namespace = "{http://www.ros.org/wiki/xacro}"
    for wheel in robot.findall(f"{xacro_namespace}drive_wheel"):
        wheel_link = wheel.attrib["name"]
        links.add(wheel_link)
        children_by_parent.setdefault("base_link", set()).add(wheel_link)

    reachable = {"base_link"}
    pending = ["base_link"]
    while pending:
        for child in children_by_parent.get(pending.pop(), set()):
            if child not in reachable:
                reachable.add(child)
                pending.append(child)

    assert links == reachable


def test_velocity_limits_are_below_firmware_limits():
    params = load_yaml("nav2_odom_params.yaml")
    planner = params["controller_server"]["ros__parameters"]["FollowPath"]
    assert planner["max_vel_x"] <= 0.24
    assert planner["max_vel_theta"] <= 1.0
    smoother = params["velocity_smoother"]["ros__parameters"]
    assert smoother["velocity_timeout"] < 0.5


def test_teleop_launch_has_no_autonomous_command_source():
    import importlib.util
    from unittest.mock import patch

    from launch_ros.actions import Node

    path = PACKAGE_ROOT / "launch" / "teleop.launch.py"
    spec = importlib.util.spec_from_file_location("teleop_launch", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    with patch.object(module, "get_package_share_directory", return_value=str(PACKAGE_ROOT)):
        description = module.generate_launch_description()
    nodes = [action for action in description.entities if isinstance(action, Node)]
    assert [node.node_package for node in nodes] == ["robot_state_publisher", "rviz2"]


def test_teleop_visualizes_raw_acceleration_not_unavailable_orientation():
    with (PACKAGE_ROOT / "rviz" / "teleop.rviz").open() as stream:
        config = yaml.safe_load(stream)["Visualization Manager"]
    assert config["Global Options"]["Fixed Frame"] == "base_link"
    displays = {display["Class"]: display for display in config["Displays"]}
    assert displays["rviz_default_plugins/Range"]["Topic"]["Value"] == "/ultrasonic/front"
    imu = displays["rviz_imu_plugin/Imu"]
    assert imu["Topic"]["Value"] == "/imu/data"
    assert not imu["fixed_frame_orientation"]
    assert not imu["Box properties"]["Enable box"]
    assert not imu["Axes properties"]["Enable axes"]
    assert imu["Acceleration properties"]["Enable acceleration"]
    assert not imu["Acceleration properties"]["Derotate acceleration"]
