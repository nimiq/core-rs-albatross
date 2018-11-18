#!/bin/sh
export RUSTFLAGS="-Clink-dead-code"

# Check for jq, cmake
if ! [ -x "$(which jq)" ]; then echo "jq not found"; exit; fi
if ! [ -x "$(which cmake)" ]; then echo "cmake not found"; exit; fi

# Guess CARGO_HOME if needed
if [ -z "$CARGO_HOME" ] && ! [ -z "$HOME" ]; then export CARGO_HOME="$HOME/.cargo"; fi

# Switch to top dir if necessary
if ! [ -e Cargo.toml ]; then cd ..; fi

# Remove old broken kcov build
if [ -e cov/kcov-master ] && ! [ -x cov/kcov-master/build/src/kcov ]; then rm -rf cov/kcov-master; fi

# New kcov build
if ! [ -e cov/kcov-master ]; then
    mkdir -p cov &&
    cd cov &&
    wget https://github.com/SimonKagstrom/kcov/archive/master.tar.gz -q &&
    tar xzf master.tar.gz &&
    mkdir -p kcov-master/build &&
    cd kcov-master/build &&
    cmake .. &&
    make &&
    cd ../../.. &&
    rm cov/master.tar.gz
fi

# Remove old results
rm -rf cov/cov

# Compile tests
tests="$(cargo test --target-dir cov --no-run --message-format=json $* | jq -r 'if .reason == "compiler-artifact" and .profile.test then .filenames[0] else "" end' | sed '/^$/d')"

# Execute tests
for file in $tests; do
    echo Test: $file;
    if [ -x $file ] && [ -f $file ]; then
        mkdir -p "cov/cov/$(basename $file)" &&
        cov/kcov-master/build/src/kcov --exclude-line="unreachable!" --exclude-pattern=$CARGO_HOME,/usr,$PWD/tests --verify "cov/cov/$(basename $file)" "$file"
    fi
done

# Merge results
cov/kcov-master/build/src/kcov --exclude-line="unreachable!" --merge cov/cov cov/cov/*

echo "Coverage:" $(grep -Po 'covered":.*?[^\\]"' cov/cov/index.js | grep "[0-9]*\.[0-9]" -o)
echo "Report: file://$PWD/cov/cov/index.html"
