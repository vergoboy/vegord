#!/bin/sh
set -e

docker build -t libvegord-builder -f Dockerfile .

docker run --rm -v "$PWD":/src -w /src libvegord-builder bash -c "
  set -e

  echo '=== Building x64 ==='
  npx node-gyp rebuild --arch=x64
  mv build/Release/vegord.node prebuilds/vegord-x64.node

  echo '=== Building arm64 ==='
  export CXX=aarch64-linux-gnu-g++
  npx node-gyp rebuild --arch=arm64
  mv build/Release/vegord.node prebuilds/vegord-arm64.node
"