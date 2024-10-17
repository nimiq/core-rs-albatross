set -e

cd extras
yarn install

# Build for bundler
yarn tsup lib/index.ts --format esm --platform browser --out-dir ../dist/lib/bundler *.ts
sed --in-place='' --expression='s/"@nimiq\/core"/"..\/..\/bundler\/index.js"/g' ../dist/lib/bundler/index.mjs

# Build for web
yarn tsup lib/index.ts --format esm --platform browser --out-dir ../dist/lib/web *.ts
sed --in-place='' --expression='s/"@nimiq\/core"/"..\/..\/web\/index.js"/g' ../dist/lib/web/index.mjs

# Build for node
yarn tsup lib/index.ts --format esm,cjs --platform node --out-dir ../dist/lib/node *.ts
sed --in-place='' --expression='s/"@nimiq\/core"/"..\/..\/nodejs\/index.js"/g' ../dist/lib/node/index.js
sed --in-place='' --expression='s/"@nimiq\/core"/"..\/..\/nodejs\/index.mjs"/g' ../dist/lib/node/index.mjs

# Build types
yarn tsup lib/index.ts --format cjs --platform node --out-dir ../dist/lib *.ts --dts-only
