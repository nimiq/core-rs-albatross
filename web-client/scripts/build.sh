#!/bin/bash

set -e

# TODO: Disable rust compiler warnings with [lints] table in Cargo.toml when it becomes stable in 1.75 (https://github.com/rust-lang/cargo/issues/12115)

# Defaults
CARGO_PROFILE="release-wasm"
CARGO_TARGET="wasm32-unknown-unknown"
TARGETS="bundler,web,nodejs,types"
RUN_WASM_OPT=true
BUILD_LAUNCHER=true
BUILD_LIB=true

# Parse command line arguments - all arguments are optional
while [[ "$#" -gt 0 ]]; do
    case $1 in
        --cargo-profile) CARGO_PROFILE="$2"; shift ;;
        --cargo-target) CARGO_TARGET="$2"; shift ;;
        --cargo-dir) CARGO_OUTPUT="$2/nimiq_web_client.wasm"; shift ;;
        -o|--only) TARGETS="$2"; shift ;;
        --skip-wasm-opt) RUN_WASM_OPT=false ;;
        --skip-launcher) BUILD_LAUNCHER=false ;;
        --skip-lib) BUILD_LIB=false ;;
        *) echo "Unknown argument: $1"; exit 1 ;;
    esac
    shift
done

CARGO_OUTPUT=${CARGO_OUTPUT:-"../target/$CARGO_TARGET/$CARGO_PROFILE/nimiq_web_client.wasm"}

# Compile web-client to WASM for a given feature set
# $1: features to enable
function compile() {
    FEATURES=$1

    echo "Compiling WASM for $FEATURES..."

    # Compile web-client to WASM
    cargo build --profile "$CARGO_PROFILE" --target "$CARGO_TARGET" --no-default-features --features "$FEATURES"
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

    echo "Generating $TYPE WASM for target $TARGET..."

    # Generate JS bindings (also generates a GC'd WASM file)
    wasm-bindgen --weak-refs --target "$TARGET" --out-name index --out-dir "$OUT_DIR" --no-typescript "$CARGO_OUTPUT"

    if [ "$RUN_WASM_OPT" = "true" ]; then
        echo "Optimizing $TYPE WASM..."
        # Optimize bindgen's WASM output for size
        wasm-opt "$BINDGEN_OUTPUT" -Os -o "$OPT_OUTPUT"
        # Replace bindgen's WASM output with the optimized one
        mv "$OPT_OUTPUT" "$BINDGEN_OUTPUT"
    fi
}

# Prepare build environment
LOCKED_WASM_BINDGEN_VERSION=$(awk '
    $0 == "name = \"wasm-bindgen\"" { found = 1; next }
    found && /^version = / { gsub(/version = "|"/, ""); print; exit }
' ../Cargo.lock)

if [ -z "$LOCKED_WASM_BINDGEN_VERSION" ]; then
    echo "Error: could not resolve wasm-bindgen version from ../Cargo.lock" >&2
    exit 1
fi

if ! command -v wasm-bindgen &> /dev/null; then
    echo "Error: wasm-bindgen-cli is not installed." >&2
    echo "Install the version pinned in Cargo.lock with:" >&2
    echo "    cargo install --locked wasm-bindgen-cli@$LOCKED_WASM_BINDGEN_VERSION" >&2
    exit 1
fi

INSTALLED_WASM_BINDGEN_VERSION=$(wasm-bindgen --version | awk '{print $2}')
if [ "$INSTALLED_WASM_BINDGEN_VERSION" != "$LOCKED_WASM_BINDGEN_VERSION" ]; then
    echo "Error: wasm-bindgen-cli version mismatch." >&2
    echo "  Cargo.lock pins:  $LOCKED_WASM_BINDGEN_VERSION" >&2
    echo "  Installed:        $INSTALLED_WASM_BINDGEN_VERSION" >&2
    echo "Reinstall with:" >&2
    echo "    cargo install --locked wasm-bindgen-cli@$LOCKED_WASM_BINDGEN_VERSION" >&2
    exit 1
fi
if ! command -v wasm-opt &> /dev/null
then
    BINARYEN_VERSION=129
    BINARYEN_FOLDER="binaryen-version_$BINARYEN_VERSION"

    # Determine the binaryen release asset for the current platform
    case "$(uname -s)" in
        Linux)  BINARYEN_OS="linux" ;;
        Darwin) BINARYEN_OS="macos" ;;
        *) echo "Error: no automatic wasm-opt download for OS '$(uname -s)'." >&2
           echo "Install binaryen $BINARYEN_VERSION manually and put wasm-opt on your PATH." >&2
           exit 1 ;;
    esac

    case "$(uname -m)" in
        x86_64|amd64) BINARYEN_ARCH="x86_64" ;;
        # binaryen names its arm64 assets "arm64" for macOS but "aarch64" for linux
        arm64|aarch64) [ "$BINARYEN_OS" = "macos" ] && BINARYEN_ARCH="arm64" || BINARYEN_ARCH="aarch64" ;;
        *) echo "Error: no automatic wasm-opt download for architecture '$(uname -m)'." >&2
           echo "Install binaryen $BINARYEN_VERSION manually and put wasm-opt on your PATH." >&2
           exit 1 ;;
    esac

    BINARYEN_ASSET="$BINARYEN_FOLDER-$BINARYEN_ARCH-$BINARYEN_OS"

    # Test if wasm-opt is already downloaded
    if [ -f "$BINARYEN_FOLDER/bin/wasm-opt" ]; then
        export PATH="$PATH:$(pwd)/$BINARYEN_FOLDER/bin"
    else
        echo "Downloading wasm-opt $BINARYEN_VERSION ($BINARYEN_ARCH-$BINARYEN_OS)..."
        # Download release from GitHub
        curl "https://github.com/WebAssembly/binaryen/releases/download/version_$BINARYEN_VERSION/$BINARYEN_ASSET.tar.gz" --location --output binaryen.tar.gz
        # Extract archive
        tar -xzf binaryen.tar.gz
        # Delete archive
        rm binaryen.tar.gz
        # Keep bin/ and lib/ together: the macOS wasm-opt resolves libbinaryen.dylib via @loader_path/../lib
        rm -rf "./$BINARYEN_FOLDER/include"
        export PATH="$PATH:$(pwd)/$BINARYEN_FOLDER/bin"
    fi
fi

contains() { case $2 in *$1* ) return 0;; *) return 1;; esac ;}

# Primitives
compile "primitives"
if contains "bundler" "$TARGETS"; then
    generate "main" "bundler"
fi
if contains "web" "$TARGETS"; then
    generate "main" "web"
fi
if contains "nodejs" "$TARGETS"; then
    generate "main" "nodejs"
fi

# Client
compile "client"
if contains "bundler" "$TARGETS"; then
    generate "worker" "bundler"
fi
if contains "web" "$TARGETS"; then
    generate "worker" "no-modules" "./dist/web/worker-wasm"
fi
if contains "nodejs" "$TARGETS"; then
    generate "worker" "nodejs"
fi

# Crypto
compile "crypto"
if contains "bundler" "$TARGETS"; then
    generate "crypto" "bundler"
fi
if contains "web" "$TARGETS"; then
    generate "crypto" "no-modules" "./dist/web/crypto-wasm"
fi
if contains "nodejs" "$TARGETS"; then
    generate "crypto" "nodejs"
fi

# Types
if contains "types" "$TARGETS"; then
    echo "Building types..."
    compile "client,crypto,primitives"
    wasm-bindgen --weak-refs --target web --out-name web --out-dir dist/types/wasm "$CARGO_OUTPUT"
    wasm-bindgen --weak-refs --target bundler --out-name bundler --out-dir dist/types/wasm "$CARGO_OUTPUT"
    find dist/types/wasm ! -name 'web.d.ts' ! -name 'bundler.d.ts' -type f -exec rm {} +
fi

# Build launcher
if [ "$BUILD_LAUNCHER" = "true" ]; then
    echo "Building launcher..."
    ./scripts/build-launcher.sh
fi

# Build lib
if [ "$BUILD_LIB" = "true" ]; then
    echo "Building lib..."
    ./scripts/build-lib.sh
fi
