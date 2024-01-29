#!/bin/bash

# Check if an argument is provided
if [ "$#" -ne 1 ]; then
    echo "Usage: $0 <pgp_key_id>"
    exit 1
fi

# Get the script's directory and change to it
SCRIPT_DIR=$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )
cd "$SCRIPT_DIR" || exit

PGP_KEY_ID="$1"
mkdir -p "../signed-digests/$PGP_KEY_ID"

SIGNATURE_DIR="../../target/digests/"

cd "$SIGNATURE_DIR" || { echo "Signature directory not found: $SIGNATURE_DIR"; exit 1; }
ls -la .

# Loop through each file in the directory
for FILE in ./*; do 
    # Check if it's a file
    if [ ! -f "$FILE" ]; then
        echo "Error: File not found - $FILE"
        continue
    fi

    FILENAME="$(basename "$FILE")"
    SIGNATURE_FILE="../../build/signed-digests/$PGP_KEY_ID/$FILENAME.DIGESTS"

    # Sign the file using the specified PGP key
    gpg --default-key "$PGP_KEY_ID" --output "$SIGNATURE_FILE" --detach-sign --clear-sign "$FILE"
done

echo "Successfully signed all digests using pgp key $PGP_KEY_ID"
