FROM ros:jazzy

ARG USERNAME=vscode
ARG USER_UID=1000
ARG USER_GID=1000

ENV DEBIAN_FRONTEND=noninteractive
ENV RUSTUP_HOME=/home/${USERNAME}/.rustup
ENV CARGO_HOME=/home/${USERNAME}/.cargo
ENV PATH="/home/${USERNAME}/.cargo/bin:${PATH}"

RUN apt-get update && apt-get install -y --no-install-recommends \
	bash \
	build-essential \
	curl \
	git \
	iproute2 \
	iputils-ping \
	jq \
	libudev-dev \
	pkg-config \
	ros-jazzy-ament-cmake-python \
	ros-jazzy-rviz2 \
	ros-jazzy-rviz-imu-plugin \
	ros-jazzy-teleop-twist-keyboard \
	ros-jazzy-ros2interface \
	ros-jazzy-rosidl-adapter \
	ros-jazzy-rosidl-cmake \
	ros-jazzy-rosidl-parser \
	ros-jazzy-tf2-tools \
	ros-jazzy-xacro \
	python3-colcon-common-extensions \
	python3-pytest \
	python3-rosdep \
	python3-yaml \
	sudo \
	&& groupmod --new-name ${USERNAME} ubuntu \
	&& usermod --login ${USERNAME} --home /home/${USERNAME} --move-home ubuntu \
	&& echo "${USERNAME} ALL=(root) NOPASSWD:ALL" > /etc/sudoers.d/${USERNAME} \
	&& chmod 0440 /etc/sudoers.d/${USERNAME} \
	&& rm -rf /var/lib/apt/lists/*

RUN mkdir -p /opt/micro_ros_agent_ws/src \
	&& git clone --depth 1 --branch jazzy \
	https://github.com/micro-ROS/micro-ROS-Agent.git \
	/opt/micro_ros_agent_ws/src/micro-ROS-Agent \
	&& rosdep update \
	&& . /opt/ros/jazzy/setup.sh \
	&& apt-get update \
	&& rosdep install --from-paths /opt/micro_ros_agent_ws/src --ignore-src -y \
	&& cd /opt/micro_ros_agent_ws \
	&& colcon build --packages-select micro_ros_agent \
	&& rm -rf /opt/micro_ros_agent_ws/src /var/lib/apt/lists/*

USER ${USERNAME}

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
	| sh -s -- -y --default-toolchain 1.94.1 \
	&& rustup target add thumbv6m-none-eabi \
	&& rustup component add rustfmt llvm-tools-preview \
	&& cargo install flip-link --version 0.1.12 \
	&& cargo install cargo-binutils \
	&& cargo install elf2uf2-rs --locked

USER root
COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

USER ${USERNAME}
WORKDIR /tenshi-no-hana
ENTRYPOINT ["/entrypoint.sh"]
CMD ["sleep", "infinity"]
