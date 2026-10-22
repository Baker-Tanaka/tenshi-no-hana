from pathlib import Path
import xml.etree.ElementTree as ET

import yaml


PACKAGE_ROOT = Path(__file__).parents[1]


def test_metadata_and_xacro_are_well_formed_xml():
    ET.parse(PACKAGE_ROOT / "package.xml")
    ET.parse(PACKAGE_ROOT / "urdf" / "tenshi_no_hana.urdf.xacro")


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


def test_runtime_dependencies_are_manual_only():
    package = ET.parse(PACKAGE_ROOT / "package.xml").getroot()
    dependencies = {element.text for element in package.findall("exec_depend")}
    assert {
        "geometry_msgs",
        "nav_msgs",
        "rclpy",
        "sensor_msgs",
        "teleop_twist_keyboard",
        "tf2_ros",
        "rviz2",
        "rviz_imu_plugin",
    } <= dependencies
    assert not any(name.startswith("nav2") for name in dependencies)
    assert sorted(path.name for path in (PACKAGE_ROOT / "launch").glob("*.py")) == [
        "teleop.launch.py"
    ]


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
    assert [node.node_package for node in nodes] == [
        "robot_state_publisher",
        "tenshi_no_hana_bringup",
        "rviz2",
    ]
    assert "exec='imu_dead_reckoning.py'" in nodes[1].cmd[0][0].describe()


def test_teleop_visualizes_raw_acceleration_not_unavailable_orientation():
    with (PACKAGE_ROOT / "rviz" / "teleop.rviz").open() as stream:
        config = yaml.safe_load(stream)["Visualization Manager"]
    assert config["Global Options"]["Fixed Frame"] == "odom"
    displays = {display["Class"]: display for display in config["Displays"]}
    assert displays["rviz_default_plugins/Range"]["Topic"]["Value"] == "/ultrasonic/front"
    imu = displays["rviz_imu_plugin/Imu"]
    assert imu["Topic"]["Value"] == "/imu/data"
    assert not imu["fixed_frame_orientation"]
    assert not imu["Box properties"]["Enable box"]
    assert not imu["Axes properties"]["Enable axes"]
    assert imu["Acceleration properties"]["Enable acceleration"]
    assert not imu["Acceleration properties"]["Derotate acceleration"]
