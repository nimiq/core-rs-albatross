set -e

cd extras
yarn install

# Build for browsers (bundler, web)
yarn tsup lib/index.ts --format esm --platform browser --out-dir ../dist/lib/browser *.ts

# Build for node
yarn tsup lib/index.ts --format esm,cjs --platform node --out-dir ../dist/lib/node *.ts

# Build types
yarn tsup lib/index.ts --format cjs --platform node --out-dir ../dist/lib *.ts --dts-only
