#!/bin/bash

# Array of all features
#features=("ChainRegistry")

features=("ChainRegistry" "Params" "TallyResults" "Pool" "Validators" "GovernanceProposalFetch" "GovernanceProposalView")


# Loop over the features
for feature in "${features[@]}"
do
   echo "Building for feature: $feature"
   # Build with --no-default-features and only the current feature
   if [ "$USE_DOCKER" = "1" ] || [ ! -f Cargo.toml ]; then
        docker run --rm -it -e RUSTFLAGS="--cfg tokio_unstable" -v "$(pwd)":/usr/src/workspace -w /usr/src/workspace rust cargo build --release --no-default-features --features "$feature EnvLogger"
   else
        RUSTFLAGS="--cfg tokio_unstable" cargo build --release --no-default-features --features "$feature EnvLogger"
   fi
   # Move the output to a .so file with the name of the feature
   mv target/release/librust_bot_plugin.so ../rust-bot/bin/lib/lib${feature}.so
done
