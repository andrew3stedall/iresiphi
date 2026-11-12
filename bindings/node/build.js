const {spawnSync} = require('node:child_process');
const {copyFileSync} = require('node:fs');
const path = require('node:path');
const args = ['build', '--locked', '--manifest-path', path.join(__dirname, 'Cargo.toml'),
  '--config', 'profile.dev.package.iresiphi.codegen-units=207'];
const result = spawnSync('cargo', args, {stdio: 'inherit'});
if (result.status !== 0) process.exit(result.status ?? 1);
copyFileSync(path.resolve(process.env.CARGO_TARGET_DIR || path.join(__dirname, 'target'), 'debug/libiresiphi_node.so'), path.join(__dirname, 'iresiphi.node'));
