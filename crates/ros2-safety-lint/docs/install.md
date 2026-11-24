# Installing rosfix

rosfix is a single executable. Pick whichever channel suits the machine.

## From PyPI

    pip install rosfix

Prebuilt wheels exist for Linux (x86_64 and aarch64), macOS and Windows. The wheel also
registers a ros2 command, so in a sourced ROS 2 shell

    ros2 rosfix src
    ros2 rosfix check src --format sarif -o rosfix.sarif

forwards its arguments to rosfix.

## From a GitHub release

Each release has a standalone archive per platform, about 15 MB, plus a SHA256SUMS file.
Unpack it and put the bin directory on PATH. The fix ranker used by `--assist` is built
in; there are no other files.

## With Docker

    docker run --rm -v "$PWD:/ws" ghcr.io/sohamwalam11/rosfix:jazzy-lite /ws

The image is based on a ROS 2 image, so installed packages are understood without further
setup. To build it yourself:

    docker build --build-arg ROS_DISTRO=jazzy -t rosfix:jazzy .

## From source

    cargo install rosfix

or, in a clone of the repository,

    cargo build --release -p rosfix

Rust 1.88 or newer is required. Optional cargo features:

    parallel   index files on several threads (--jobs N)
    online     allow --online reference lookups

## Two ways to use it

From any terminal (PowerShell, cmd, bash, zsh), one command at a time:

    rosfix check src
    rosfix fix src --write

Or open rosfix's own shell once and type the commands without the prefix:

    rosfix shell src
    rosfix:src> status
    rosfix:src> fix --write
    rosfix:src> exit

Both run the same code; pick whichever suits you. See `rosfix docs commands`.

## Checking the installation

    rosfix --version
    rosfix docs
    rosfix doctor

`rosfix doctor` reports on the ROS 2 environment of the current shell, which is a good
first step on a new machine (see `rosfix docs doctor`).
