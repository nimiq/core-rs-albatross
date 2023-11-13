set -e

cd launcher
yarn install

# Build for browsers (bundler, web)
yarn tsup --format esm --platform browser --out-dir ../dist/lib/browser *.ts

# # Build for node
# yarn tsup --format esm,cjs --platform node --out-dir ../dist/lib/node *.ts
