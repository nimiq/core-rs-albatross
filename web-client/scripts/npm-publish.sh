./scripts/build-bundler.sh &&
./scripts/build-web.sh &&
./scripts/build-types.sh &&
cd dist &&
npm publish --tag next &&
cd ..
