#!/bin/bash

MAX_VALIDATORS=4

usage()
{
cat << EOF
usage: $0 [-v|--validators COUNT] [-h|--help]

This script launches a scenario with N validators + spammer + seed node, each one running in its own docker container
This script should be executed from the root of your nimiq repository

OPTIONS:
   -h|--help       Show this message
   -v|--validators The number of validators, as a minimun 4 validators are created
EOF
}

while [ ! $# -eq 0 ]; do
    case "$1" in
        -v | --validators)
            if [ "$2" ]; then
                MAX_VALIDATORS=$2
                shift
            else
                echo '--validators requires a value'
                exit 1
            fi
            ;;
        -h | --help)
            usage
            exit
            ;;
        *)
            usage
            exit
            ;;
    esac
    shift
done

tmp_dir=`mktemp -d -t docker_devnet.XXXXXXXXXX`

# Create devnet configuration
echo "Creating devnet configuration... "
echo "[[seed]]
restartable = false
sync_mode = \"history\"

[[spammer]]
restartable = false
sync_mode = \"history\"
tpb = 15
" > $tmp_dir/topology.toml
for _ in {0..$MAX_VALIDATORS}
do
  echo "[[validator]]
restartable = true
sync_mode = \"history\"
" >> $tmp_dir/topology.toml
done
  
python3 scripts/devnet/devnet.py -t $tmp_dir/topology.toml --dry -o $tmp_dir

# Clone the albatross generator repo
git clone https://github.com/redmaner/albatross_generator.git $tmp_dir/albatross_generator --depth 1

# Overwrite the docker compose and genesis
echo "Copying the genesis and docker compose files... "

#Compile the code
echo "Compiling the code using '$tmp_dir/dev-albatross.toml' ..."
cp -v $tmp_dir/dev-albatross.toml genesis/src/genesis/dev-albatross-4-validators.toml
cargo build --release

echo "Create docker images... "
docker buildx build . --pull -t core --progress=plain --build-arg BUILD=release
docker buildx build . --pull -t spammer --progress=plain --build-arg APP=spammer --build-arg BUILD=release
docker buildx build $tmp_dir/albatross_generator --pull -t albagen --progress=plain

echo "Launching docker compose with '$tmp_dir/docker-compose.yml' as compose file ..."
NETWORK_NAME=nimiq.local docker-compose -f $tmp_dir/docker-compose.yml up
