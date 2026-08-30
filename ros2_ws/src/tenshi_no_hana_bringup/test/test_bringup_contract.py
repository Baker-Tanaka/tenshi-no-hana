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


def test_amcl_mode_uses_map_and_laser_scan():
    params = load_yaml("nav2_amcl_params.yaml")
    assert params["amcl"]["ros__parameters"]["global_frame_id"] == "map"
    assert params["amcl"]["ros__parameters"]["odom_frame_id"] == "odom"
    global_costmap = params["global_costmap"]["global_costmap"]["ros__parameters"]
    assert "static_layer" in global_costmap["plugins"]
    assert global_costmap["obstacle_layer"]["scan"]["topic"] == "/scan"


def test_velocity_limits_are_below_firmware_limits():
    for name in ("nav2_odom_params.yaml", "nav2_amcl_params.yaml"):
        params = load_yaml(name)
        planner = params["controller_server"]["ros__parameters"]["FollowPath"]
        assert planner["max_vel_x"] <= 0.24
        assert planner["max_vel_theta"] <= 1.0
        smoother = params["velocity_smoother"]["ros__parameters"]
        assert smoother["velocity_timeout"] < 0.5
