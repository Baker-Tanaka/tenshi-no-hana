#!/usr/bin/env python3

import math

from geometry_msgs.msg import TransformStamped
from nav_msgs.msg import Odometry
import rclpy
from rclpy.node import Node
from sensor_msgs.msg import Imu
from tf2_ros import TransformBroadcaster


class ImuDeadReckoning(Node):
    def __init__(self):
        super().__init__('imu_dead_reckoning')
        self.declare_parameter('calibration_samples', 100)
        self.declare_parameter('acceleration_deadband', 0.15)
        self.declare_parameter('velocity_damping', 0.2)

        self.calibration_samples = max(
            1, self.get_parameter('calibration_samples').get_parameter_value().integer_value
        )
        self.acceleration_deadband = max(
            0.0, self.get_parameter('acceleration_deadband').value
        )
        self.velocity_damping = max(0.0, self.get_parameter('velocity_damping').value)

        self.odom_publisher = self.create_publisher(Odometry, '/odom', 10)
        self.tf_broadcaster = TransformBroadcaster(self)
        self.subscription = self.create_subscription(Imu, '/imu/data', self.imu_callback, 10)

        self.calibration_count = 0
        self.accel_bias_x = 0.0
        self.accel_bias_y = 0.0
        self.gyro_bias_z = 0.0
        self.x = 0.0
        self.y = 0.0
        self.yaw = 0.0
        self.velocity_x = 0.0
        self.velocity_y = 0.0
        self.yaw_rate = 0.0
        self.last_time = None

        self.get_logger().info(
            'Starting IMU dead reckoning; keep the rover stationary for '
            f'{self.calibration_samples} samples.'
        )

    def imu_callback(self, message):
        now = self.get_clock().now()
        current_time = now.nanoseconds * 1e-9
        acceleration_x = message.linear_acceleration.x
        acceleration_y = message.linear_acceleration.y
        angular_velocity_z = message.angular_velocity.z

        if self.calibration_count < self.calibration_samples:
            count = self.calibration_count + 1
            self.accel_bias_x += (acceleration_x - self.accel_bias_x) / count
            self.accel_bias_y += (acceleration_y - self.accel_bias_y) / count
            self.gyro_bias_z += (angular_velocity_z - self.gyro_bias_z) / count
            self.calibration_count = count
            if count == self.calibration_samples:
                self.get_logger().info('IMU calibration complete.')
            self.last_time = current_time
            self.publish_state(now)
            return

        if self.last_time is not None:
            dt = current_time - self.last_time
            if 0.001 <= dt <= 0.1:
                self.yaw_rate = angular_velocity_z - self.gyro_bias_z
                self.yaw = normalize_angle(self.yaw + self.yaw_rate * dt)
                body_acceleration_x = acceleration_x - self.accel_bias_x
                body_acceleration_y = acceleration_y - self.accel_bias_y
                acceleration_magnitude = math.hypot(
                    body_acceleration_x, body_acceleration_y
                )
                if acceleration_magnitude < self.acceleration_deadband:
                    body_acceleration_x = 0.0
                    body_acceleration_y = 0.0

                cos_yaw = math.cos(self.yaw)
                sin_yaw = math.sin(self.yaw)
                acceleration_odom_x = (
                    cos_yaw * body_acceleration_x - sin_yaw * body_acceleration_y
                )
                acceleration_odom_y = (
                    sin_yaw * body_acceleration_x + cos_yaw * body_acceleration_y
                )

                damping = math.exp(-self.velocity_damping * dt)
                self.velocity_x = (
                    self.velocity_x + acceleration_odom_x * dt
                ) * damping
                self.velocity_y = (
                    self.velocity_y + acceleration_odom_y * dt
                ) * damping
                self.x += self.velocity_x * dt
                self.y += self.velocity_y * dt

        self.last_time = current_time
        self.publish_state(now)

    def publish_state(self, stamp):
        half_yaw = self.yaw * 0.5
        sin_half_yaw = math.sin(half_yaw)
        cos_half_yaw = math.cos(half_yaw)

        odometry = Odometry()
        odometry.header.stamp = stamp.to_msg()
        odometry.header.frame_id = 'odom'
        odometry.child_frame_id = 'base_link'
        odometry.pose.pose.position.x = self.x
        odometry.pose.pose.position.y = self.y
        odometry.pose.pose.orientation.z = sin_half_yaw
        odometry.pose.pose.orientation.w = cos_half_yaw
        odometry.twist.twist.linear.x = self.velocity_x
        odometry.twist.twist.linear.y = self.velocity_y
        odometry.twist.twist.angular.z = self.yaw_rate
        self.odom_publisher.publish(odometry)

        transform = TransformStamped()
        transform.header = odometry.header
        transform.child_frame_id = odometry.child_frame_id
        transform.transform.translation.x = self.x
        transform.transform.translation.y = self.y
        transform.transform.rotation = odometry.pose.pose.orientation
        self.tf_broadcaster.sendTransform(transform)


def normalize_angle(angle):
    return math.atan2(math.sin(angle), math.cos(angle))


def main(args=None):
    rclpy.init(args=args)
    node = ImuDeadReckoning()
    try:
        rclpy.spin(node)
    finally:
        node.destroy_node()
        rclpy.shutdown()


if __name__ == '__main__':
    main()
