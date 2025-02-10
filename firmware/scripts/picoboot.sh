#!/bin/bash

set -euxo pipefail

# You can find your serial using `poststation-cli ls`
SERIAL="19824B024328F5CA"

# Move to bootloader mode
poststation-cli \
    device \
    $SERIAL\
    proxy \
    reset 

# flash the device
cargo run --release