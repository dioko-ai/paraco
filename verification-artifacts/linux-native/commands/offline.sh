#!/bin/bash
set -euo pipefail
source /tmp/paraco-native-tools/env.sh
out=verification-artifacts/linux-native
binary=/tmp/paraco-native-tools/fixed/paraco-0.1.0-x86_64-unknown-linux-gnu/bin/paraco
"$binary" prepare examples/offline-dependency --output /tmp/paraco-native-tools/prepared --deno /tmp/paraco-native-tools/deno/deno
cp /tmp/paraco-native-tools/bundle-fixed/SHA256SUMS "$out/bundle-fixed-sha256.txt"
cp /tmp/paraco-native-tools/fixed/paraco-*/bundle.json "$out/bundle-fixed.json"
find /tmp/paraco-native-tools/prepared -type f -exec sha256sum {} + | sort > "$out/prepared-before.sha256"
mkdir /tmp/paraco-native-tools/invalid-app
printf '{"name":"invalid","entrypoint":"main.ts","capabilities":[]}' > /tmp/paraco-native-tools/invalid-app/paraco.json
printf "import 'unsupported:failed-update'; export default {}" > /tmp/paraco-native-tools/invalid-app/main.ts
if "$binary" prepare /tmp/paraco-native-tools/invalid-app --output /tmp/paraco-native-tools/failed-revision --deno /tmp/paraco-native-tools/deno/deno; then exit 1; fi
test ! -e /tmp/paraco-native-tools/failed-revision
find /tmp/paraco-native-tools/prepared -type f -exec sha256sum {} + | sort > "$out/prepared-after.sha256"
cmp "$out/prepared-before.sha256" "$out/prepared-after.sha256"
docker pull ubuntu:24.04
docker image inspect ubuntu:24.04 --format '{{json .RepoDigests}} {{.Architecture}}' > "$out/clean-image.txt"
docker run --rm --name paraco-native-offline --network none --mount type=bind,src=/tmp/paraco-native-tools/bundle-fixed,dst=/input,readonly --mount type=bind,src=/tmp/paraco-native-tools/prepared,dst=/relocated-prepared,readonly --mount type=bind,src="$PWD/examples/hello",dst=/hello,readonly --mount type=bind,src="$PWD/tests/clean-offline.sh",dst=/clean-offline.sh,readonly ubuntu:24.04 bash -c 'set -e; cat /etc/os-release; getconf GNU_LIBC_VERSION; echo network_interfaces:; ls /sys/class/net; cd /input; sha256sum -c SHA256SUMS; mkdir /opt/relocated; tar -xzf /input/*.tar.gz -C /opt/relocated; bash /clean-offline.sh /opt/relocated/paraco-0.1.0-x86_64-unknown-linux-gnu/bin/paraco /relocated-prepared /hello'
find /tmp/paraco-native-tools/prepared -type f -exec sha256sum {} + | sort > "$out/prepared-after-launch.sha256"
cmp "$out/prepared-before.sha256" "$out/prepared-after-launch.sha256"
echo 'Failed new revision and offline launches preserved every installed artifact digest.'
