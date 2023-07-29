# rust-bot-bundle
A modular rewrite of Cosmos-Rust-Bot



## Prerequisites

This project provides a convenient way to set up a Rust development environment using Docker. By leveraging Docker, you can work with Rust and its package manager, Cargo, without installing them directly on your host system. Docker will handle the Rust toolchain within a container, keeping your system clean and isolated.

Before you begin, make sure you have Docker installed on your system. 

## Getting Started

Run the following command to pull the official Rust Docker image:

```bash
docker pull rust
```
Create an Alias for Cargo:

```bash
 alias cargo='docker run --rm -it -e RUSTFLAGS="--cfg tokio_unstable" -v "$(pwd)":/usr/src/workspace -w /usr/src/workspace rust cargo'
```

## Build the plugins
```bash 
cd rust-bot-plugin
USE_DOCKER=1  ./build.sh
```

## Run the bot
```bash 
cd ../rust-bot
cargo run
```
