#!/bin/bash

SETUP_INSTRUCTION_MESSAGE="Please download all signed digests from the Nimiq GitHub release page and place them in core-rs-albatross/signed-digests."

set -e

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" &> /dev/null && pwd)
cd "$SCRIPT_DIR" || { echo "Failed to change directory to $SCRIPT_DIR"; exit 1; }
TARGET_DIR="../signed-digests"

# Function to check if a key's trust level is acceptable
is_valid_key() {
    local key_id=$1  

    # Check if the key is present in the local keyring
    if ! gpg --list-keys "$key_id" &>/dev/null; then
        echo "Key not found in keyring: $key_id"
        return 1
    fi

    return 0
}

SIGNED_FILE_FOUND=false

# Function to process files
process_file() {
    local file=$1
    if [ -f "$file" ]; then
        echo "Processing file: $file"
        SIGNED_FILE_FOUND=true
        # Check the file's PGP signature
        if gpg --verify "$file" &>/dev/null; then
            # Extract the key ID from the verification message
            key_id=$(gpg --verify "$file" 2>&1 | grep -oP 'using RSA key \K[A-F0-9]+')
            if ! is_valid_key "$key_id"; then
                echo "Signature from an unknown key found in $file. Please ensure you have Nimiq team's public PGP keys in your local GPG keychain."
                exit 1
            fi
        else
            echo "============================================="
            echo "WARNING: Invalid signature found in $file"
            echo "============================================="
            exit 1
        fi
    fi
}

# Check files directly in the target directory
for file in "$TARGET_DIR"/*; do
    process_file "$file"
done

# Check files in subdirectories of the target directory
for subdir in "$TARGET_DIR"/*; do
    if [ -d "$subdir" ]; then
        for file in "$subdir"/*; do
            process_file "$file"
        done
    fi
done

if [ "$SIGNED_FILE_FOUND" = false ]; then
    echo "No signed files found in $TARGET_DIR or its subdirectories. $SETUP_INSTRUCTION_MESSAGE"
    exit 1
fi

echo "Signature checks complete."
