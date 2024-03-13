#!/bin/bash

set -e

SIGNED_DIGESTS_INSTRUCTION_MESSAGE="Please download all signed digests from the Nimiq GitHub release page and place them in core-rs-albatross/signed-digests."
DIGESTS_INSTRUCTION_MESSAGE="Please make sure you have built or downloaded the latest release, and that it's present: core-rs-albatross/target."

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" &> /dev/null && pwd)
cd "$SCRIPT_DIR" || { echo "Failed to change directory to $SCRIPT_DIR"; exit 1; }

# Define directories
DIGESTS_DIR="../../target/digests"
SIGNED_DIGESTS_DIR="../signed-digests"

# Check if the digests and signed-digests directories exist
if [ ! -d "$DIGESTS_DIR" ]; then
    echo "Digests directory not found: $DIGESTS_DIR. $DIGESTS_INSTRUCTION_MESSAGE"
    exit 1
fi

# Check if there are files in the digests directory
DIGEST_FILES=$(find "$DIGESTS_DIR" -maxdepth 1 -type f)
if [ -z "$DIGEST_FILES" ]; then
    echo "No files found in digests directory: $DIGESTS_DIR. $DIGESTS_INSTRUCTION_MESSAGE"
    exit 1
fi

# Check if there is a dir for signed digests
if [ ! -d "$SIGNED_DIGESTS_DIR" ]; then
    echo "Signed-digests directory not found: $SIGNED_DIGESTS_DIR. $SIGNED_DIGESTS_INSTRUCTION_MESSAGE"
    exit 1
fi


# Iterate over each digest file in the digests directory
for DIGEST_FILE in $DIGEST_FILES; do
    FILENAME="$(basename "$DIGEST_FILE")"
    FILE_FOUND=false

    # Extract only the hash values from the DIGEST_FILE into a temporary file
    TEMP_DIGEST_FILE=$(mktemp)
    grep -Eo '^[a-f0-9]{32,}' "$DIGEST_FILE" > "$TEMP_DIGEST_FILE"

    # Search in signed-digests and its subdirectories for files that match the digest filename
    for SIGNED_FILE in "$SIGNED_DIGESTS_DIR"/*/"$FILENAME"*; do
        if [ -f "$SIGNED_FILE" ]; then
            FILE_FOUND=true
            # Extract only the hash values from the PGP signed file into a temporary file
            TEMP_SIGNED_FILE=$(mktemp)
            sed -n '/-----BEGIN PGP SIGNED MESSAGE-----/,/-----BEGIN PGP SIGNATURE-----/p' "$SIGNED_FILE" | grep -Eo '^[a-f0-9]{32,}' > "$TEMP_SIGNED_FILE"

            # Compare contents of the two temporary files
            if ! cmp -s "$TEMP_DIGEST_FILE" "$TEMP_SIGNED_FILE"; then
                echo "============================================="
                echo "WARNING: Hash mismatch found for $SIGNED_FILE"
                echo "============================================="
                rm "$TEMP_SIGNED_FILE"
                rm "$TEMP_DIGEST_FILE"
                exit 1
            fi

            # Clean up temporary files
            rm "$TEMP_SIGNED_FILE"
        fi
    done

    if [ "$FILE_FOUND" = false ]; then
        echo "No matching signed file found for $DIGEST_FILE. $DIGESTS_INSTRUCTION_MESSAGE"
        rm "$TEMP_DIGEST_FILE"
        exit 1
    fi

    # Clean up temporary digest file
    rm "$TEMP_DIGEST_FILE"
done

echo "Hash checks complete."
