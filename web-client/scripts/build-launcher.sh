set -e

cd extras
yarn install

# Build for browsers (bundler, web)
yarn tsup launcher --format esm --platform browser --out-dir ../dist/launcher/browser *.ts

# Build for node
yarn tsup launcher --format esm,cjs --platform node --out-dir ../dist/launcher/node *.ts
