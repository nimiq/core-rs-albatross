set -e
./scripts/build-bundler.sh
./scripts/build-web.sh
./scripts/build-node.sh
./scripts/build-types.sh
./scripts/build-launcher.sh
cd dist
npm publish --tag next
cd ..
