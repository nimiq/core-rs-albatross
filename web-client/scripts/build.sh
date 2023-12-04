set -e

# TODO: Disable rust compiler warnings with [lints] table in Cargo.toml when it becomes stable in 1.75 (https://github.com/rust-lang/cargo/issues/12115)

CARGO_PROFILE="release-wasm"
CARGO_TARGET="wasm32-unknown-unknown"
CARGO_OUTPUT="../target/$CARGO_TARGET/$CARGO_PROFILE/nimiq_web_client.wasm"

# Compile web-client to WASM for a given feature set
# $1: features to enable
function compile() {
    FEATURES=$1

    printf "Compiling WASM for $FEATURES...\n"

    # Compile web-client to WASM
    cargo build --profile $CARGO_PROFILE --target $CARGO_TARGET --no-default-features --features $FEATURES
}

# Generate WASM for a given type and target
# $1: Type (main, worker)
# $2: Target (bundler, web, nodejs, no-modules, deno)
# $3: Output directory (optional, default: ./dist/$2/$1-wasm)
function generate() {
    TYPE=$1
    TARGET=$2
    OUT_DIR=${3:-"./dist/$TARGET/$TYPE-wasm"}

    BINDGEN_OUTPUT="$OUT_DIR/index_bg.wasm"
    OPT_OUTPUT="$BINDGEN_OUTPUT.opt"

    printf "Generating $TYPE WASM for target $TARGET...\n"

    # Generate JS bindings (also generates a GC'd WASM file)
    wasm-bindgen --weak-refs --target $TARGET --out-name index --out-dir $OUT_DIR --no-typescript $CARGO_OUTPUT
    # Optimize bindgen's WASM output for size
    wasm-opt -Os -o $OPT_OUTPUT $BINDGEN_OUTPUT
    # Replace bindgen's WASM output with the optimized one
    mv $OPT_OUTPUT $BINDGEN_OUTPUT
}

# Prepare build environment
printf "Preparing build environment...\n"
cargo install wasm-bindgen-cli wasm-opt

# Primitives
compile "primitives"
generate "main" "bundler"
generate "main" "web"
generate "main" "nodejs"

# Client
compile "client"
generate "worker" "bundler"
generate "worker" "no-modules" "./dist/web/worker-wasm"
generate "worker" "nodejs"

# Types
printf "Building types...\n"
compile "client,primitives"
wasm-bindgen --weak-refs --target web --out-name web --out-dir dist/types $CARGO_OUTPUT
wasm-bindgen --weak-refs --target bundler --out-name bundler --out-dir dist/types $CARGO_OUTPUT
find dist/types ! -name 'web.d.ts' ! -name 'bundler.d.ts' -type f -exec rm {} +

# Build launcher
printf "Building launcher...\n"
./scripts/build-launcher.sh
