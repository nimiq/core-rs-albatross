set -e

./scripts/build.sh

cd dist
npm publish --tag next
cd ..
