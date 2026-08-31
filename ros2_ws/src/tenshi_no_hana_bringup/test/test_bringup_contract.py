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
    assert "navigation_launch.py" in launch_source
    for stale_name in ("amcl", "localization_mode", "localization_launch.py", "scan_topic"):
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
