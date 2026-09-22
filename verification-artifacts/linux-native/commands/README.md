# Recorded commands

Run from repository root. Scripts use `/tmp/paraco-native-tools` and preserve
exit statuses in `exit-statuses.txt`. They describe this run, not an idempotent
installer. `install.sh` initially failed because unzip was absent; recovery was:

```sh
python3 -m zipfile -e /tmp/paraco-native-tools/deno.zip /tmp/paraco-native-tools/deno
chmod +x /tmp/paraco-native-tools/deno/deno
```

Official download URLs are in `install.sh`. Rust was already installed through
rustup; binary hashes/components are in the environment records. Browser
artifacts were already cached under `/tmp/paraco-playwright`; pinned Playwright
installed/checked them successfully and Chromium executed in the complete suite.

After fixing systemd rendering, the build command from `bundle.sh` was repeated
with output `/tmp/paraco-native-tools/bundle-fixed`, extracted under
`/tmp/paraco-native-tools/fixed`. Final checks:

```sh
source /tmp/paraco-native-tools/env.sh
npm run check
cargo test --bin paraco service::tests
python3 scripts/verify-prepared.py
PARACO_BIN=/tmp/paraco-native-tools/fixed/paraco-0.1.0-x86_64-unknown-linux-gnu/bin/paraco python3 scripts/verify-service.py
PARACO_BIN=/tmp/paraco-native-tools/fixed/paraco-0.1.0-x86_64-unknown-linux-gnu/bin/paraco python3 scripts/verify-linux-durable.py
bash /tmp/paraco-native-tools/offline.sh
tests/bundle-smoke.sh --deno /tmp/paraco-native-tools/deno/deno --deno-notice /tmp/paraco-native-tools/fixture-notice --third-party-notice /tmp/paraco-native-tools/fixture-notice --target x86_64-unknown-linux-gnu
```

`source-diff.patch` contains changes to tracked implementation/test files;
`source-sha256.txt` also fingerprints the new `scripts/verify-linux-durable.py`.
`working-tree-diff.sha256` fingerprints the final tracked documentation diff too.
No live bearer URLs are retained.
