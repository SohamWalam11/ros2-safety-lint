# rosfix inside a ROS 2 image, so installed packages are understood out of the box.
#   docker build --build-arg ROS_DISTRO=jazzy -t rosfix:jazzy .
#   docker run --rm -v "$PWD:/ws" rosfix:jazzy /ws --format sarif -o /ws/rosfix.sarif
ARG ROS_DISTRO=jazzy

FROM rust:1.82-bookworm AS build
WORKDIR /src
COPY Cargo.toml ./
COPY crates ./crates
RUN cargo build --release -p rosfix --locked || cargo build --release -p rosfix

FROM ros:${ROS_DISTRO}-ros-base
COPY --from=build /src/target/release/rosfix /opt/rosfix/bin/rosfix
# Source the ROS environment so AMENT_PREFIX_PATH / ROS_DISTRO are set for rosfix.
RUN printf '#!/bin/bash\nsource /opt/ros/${ROS_DISTRO}/setup.bash\nexec /opt/rosfix/bin/rosfix "$@"\n' > /usr/local/bin/rosfix-entry \
 && chmod +x /usr/local/bin/rosfix-entry
WORKDIR /ws
ENTRYPOINT ["/usr/local/bin/rosfix-entry"]
CMD ["/ws"]
