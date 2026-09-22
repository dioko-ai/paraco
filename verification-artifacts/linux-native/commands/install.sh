#!/bin/bash
set -euo pipefail
cd /tmp/paraco-native-tools
curl -fL --retry 2 -o node.tar.xz https://nodejs.org/dist/v22.14.0/node-v22.14.0-linux-x64.tar.xz
curl -fL --retry 2 -o node-SHASUMS256.txt https://nodejs.org/dist/v22.14.0/SHASUMS256.txt
expected=$(awk '$2=="node-v22.14.0-linux-x64.tar.xz"{print $1}' node-SHASUMS256.txt)
echo "$expected  node.tar.xz" | sha256sum -c -
tar -xf node.tar.xz
curl -fL --retry 2 -o deno.zip https://github.com/denoland/deno/releases/download/v2.2.5/deno-x86_64-unknown-linux-gnu.zip
mkdir -p deno
unzip -o deno.zip -d deno
sha256sum node.tar.xz deno.zip deno/deno
