#!/usr/bin/env node
const { spawnSync } = require("child_process");
const path = require("path");

const r = spawnSync(path.join(__dirname, "runway-bin"), process.argv.slice(2), {
  stdio: "inherit",
});
process.exit(r.status ?? 1);
