#!/bin/bash

max_restarts=10
# Initializing variables
validators=(1 2 3 4)
pids=()
user="$USER"
fail=false
foldername=$(date +%Y%m%d_%H%M%S)
ERASE=false
DATABASE_CLEAN=false
CONTINOUS=false

trap cleanup_exit INT

function cleanup_exit() {
    echo "Killing all validators and finishing...."
    for pid in ${pids[@]}; do
        kill $pid
    done
    if [ "$fail" = true ] ; then
        echo "...FAILED..."
        exit 1
    fi
    exit 0
}

usage()
{
cat << EOF
usage: $0 [-r|--restarts COUNT] [-e|--erase] [-h|--help]

This script launches 4 validators and restarts them

OPTIONS:
   -h|--help      Show this message
   -e|--erase     Erases all of the validator state as part of restarting it
   -d|--db        Erases only the database state of the validator as part of restarting it 
   -r|--restarts  The number of times you want to kill/restart validators (by default 10 times)(0 means no restarts)
   -c|--continous In continous mode the script runs until it is killed (or it finds an error)
EOF
}

while [ ! $# -eq 0 ]; do
    case "$1" in
        -r | --restarts)
            if [ "$2" ]; then
                max_restarts=$2
                shift
            else
                echo '--restarts requires a value'
                exit 1
            fi
            ;;
        -e | --erase)
            ERASE=true
            ;;
        -c | --continous)
            CONTINOUS=true
            ;;
        -d | --db)
            DATABASE_CLEAN=true
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

#Create directory for logs
mkdir -p  temp-logs/"$foldername"

# Erase all previous state (if any) and start fresh
rm -rf temp-state
mkdir -p temp-state/dev/{1,2,3,4}

#Launch the validators and store their PID
echo "Starting validators.... "
for validator in ${validators[@]}; do
    cargo run --bin nimiq-client -- -c configs/dev/dev-$validator.toml &>> temp-logs/$foldername/Validator$validator.txt &
    pids+=($!)
    sleep 1s
done
echo "Done"

#Let the validators produce blocks for 30 seconds
sleep 30s

old_block_number=0
restarts_count=0

cycles=0
while [ $cycles -le $max_restarts ]
do

    if [ $restarts_count -lt $max_restarts ] ; then

        #Select a random validator to restart
        index=$((0 + $RANDOM % 3))

        echo "  Killing validator: $(($index + 1 ))"

        kill ${pids[$index]}
        sleep 10s

        if [ "$ERASE" = true ] ; then
            echo "  Erasing all validator state"
            rm -rf temp-state/dev/$(($index + 1 ))/*
            echo "################################## VALIDATOR STATE DELETED ###########################  " >> temp-logs/$foldername/Validator$(($index + 1 )).txt
        fi

        if [ "$DATABASE_CLEAN" = true ] ; then
            echo "  Erasing validator database"
            rm -rf temp-state/dev/$(($index + 1 ))/devalbatross-history-consensus
            echo "################################## VALIDATOR DB DELETED ###########################  " >> temp-logs/$foldername/Validator$(($index + 1 )).txt
        fi

        echo "################################## RESTART ###########################  " >> temp-logs/$foldername/Validator$(($index + 1 )).txt

        echo "  Restarting validator: $(($index + 1 ))"
        cargo run --bin nimiq-client -- -c configs/dev/dev-$(($index + 1 )).toml &>> temp-logs/$foldername/Validator$(($index + 1 )).txt &
        pids[$index]=$!
        restarts_count+=1
    fi

    if [ "$CONTINOUS" = false ] ; then
        cycles=$(( $cycles + 1 ))
    fi

    sleep_time=$((30 + $RANDOM % 150))

    #Produce blocks for some minutes
    echo "  Producing blocks for $sleep_time seconds"
    sleep "$sleep_time"s

    #Search for deadlocks
    if grep -wrin "deadlock" temp-logs/$foldername/
    then
        echo "   !!!!   DEADLOCK   !!! "
        fail=true
        break
    fi

    #Search for panics/crashes
    if grep -wrin "panic" temp-logs/$foldername/
    then
        echo "   !!!!   PANIC   !!! "
        fail=true
        break
    fi
    #Search if blocks are being produced
    bns=()

    #First collect the last block number from each validator
    for log in temp-logs/$foldername/*; do
        bn=$(grep "Now at block #" $log | tail -1 | awk -F# '{print $2}')
        if [ -z "$bn" ]; then
            bns+=(0)
        else
            bns+=($bn)
        fi
    done

    # Obtain the greatest one
    new_block_number=0
    for n in "${bns[@]}" ; do
        ((n > new_block_number)) && new_block_number=$n
    done

    echo "     Latest block number: $new_block_number "

    if [ $new_block_number -le $old_block_number ] ; then
        echo "   !!!!   BLOCKS ARE NOT BEING PRODUCED AFTER $sleep_time seconds   !!! "
        fail=true
        break
    fi
    old_block_number=$new_block_number

done

sleep 30s

cleanup_exit