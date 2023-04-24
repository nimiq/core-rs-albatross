./scripts/build-bundler.sh &&
./scripts/build-web.sh &&
cd dist &&
npm publish --tag next &&
cd ..
