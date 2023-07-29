#!/bin/bash

# Array of all features
features=("ChainRegistry")

#features=("ChainRegistry" "Params" "TallyResults" "Pool" "Validators" "GovernanceProposalFetch" "GovernanceProposalView")

# Loop over the features
for feature in "${features[@]}"
do
   echo "Building for feature: $feature"
   # Build with --no-default-features and only the current feature
   cargo build --release --no-default-features --features "$feature EnvLogger"
   # Move the output to a .so file with the name of the feature
   mv target/release/librust_bot_plugin.so ../rust-bot/bin/lib/lib${feature}.so
done
