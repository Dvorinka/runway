#!/usr/bin/env node
// Downloads the matching release binary from GitHub releases.
const { execSync, spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const https = require("https");

const REPO = "Dvorinka/runway";
const { version } = require("./package.json");

const TARGETS = {
  "linux-x64": "linux-x64",
  "linux-arm64": "linux-arm64",
  "darwin-x64": "darwin-x64",
  "darwin-arm64": "darwin-arm64",
};

const target = TARGETS[`${process.platform}-${process.arch}`];
if (!target) {
  console.error(`runway: unsupported platform ${process.platform}-${process.arch}`);
  process.exit(1);
}

const url = `https://github.com/${REPO}/releases/download/v${version}/runway-${target}.tar.gz`;
const dest = path.join(__dirname, "runway-bin");

function download(url, out, redirects = 5) {
  return new Promise((resolve, reject) => {
    https
      .get(url, { headers: { "user-agent": "runway-npm" } }, (res) => {
        if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location && redirects) {
          return resolve(download(res.headers.location, out, redirects - 1));
        }
        if (res.statusCode !== 200) {
          return reject(new Error(`download failed: HTTP ${res.statusCode}`));
        }
        res.pipe(out).on("finish", () => out.close(resolve));
      })
      .on("error", reject);
  });
}

(async () => {
  const tarball = path.join(__dirname, "runway.tgz");
  await download(url, fs.createWriteStream(tarball));
  execSync(`tar -xzf ${JSON.stringify(tarball)} -C ${JSON.stringify(__dirname)}`);
  fs.renameSync(path.join(__dirname, "runway"), dest);
  fs.unlinkSync(tarball);
  fs.chmodSync(dest, 0o755);
  const probe = spawnSync(dest, ["--version"], { encoding: "utf8" });
  if (probe.status !== 0) {
    console.error(`runway: binary self-check failed: ${probe.stderr || probe.stdout}`);
    process.exit(1);
  }
})().catch((e) => {
  console.error(`runway: ${e.message}`);
  process.exit(1);
});
