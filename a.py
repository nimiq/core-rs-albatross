from collections import defaultdict
from datetime import datetime, timedelta
import sys

INTERESTING = [
    "do_push:",
    "extend:",
    "verify:",
    "verify_block_body:macro:",
    "verify_block_body:micro:",
    "verify_block_justification:macro:",
    "verify_block_justification:micro:",
]

def main():
    import argparse
    p = argparse.ArgumentParser(description="Try to extract some aggregate timings from logs")
    p.add_argument("files", metavar="FILE", nargs="+", help="Log files to scan")
    args = p.parse_args()

    previous_timestamps = {}
    durations = defaultdict(list)
    bad_durations = {"": defaultdict(list), "micro": defaultdict(list), "macro": defaultdict(list)}
    cur_durations = {}

    for file in args.files:
        with open(file) as f:
            for line in f:
                split = line.split(" | ", 1)
                if len(split) < 2:
                    continue
                prefix = split[0].strip()
                message = split[1].strip()

                found = 0
                if message.startswith("lock held for a long time:"):
                    for needle, result in bad_durations.items():
                        if any(needle in message for message in cur_durations):
                            found += 1
                            for message, dur in cur_durations.items():
                                result[message].append(dur)
                if found not in [0, 2]:
                    raise ValueError(found)

                name = None
                number = None
                for interesting in INTERESTING:
                    if message.startswith(interesting):
                        name = interesting
                        number = message[len(interesting):]
                        break
                if not name:
                    continue
                if message == "do_push:0":
                    cur_durations = {}
                date, timeofday, rest = prefix.split(" ", 2)
                time = datetime.fromisoformat("{} {}".format(date, timeofday))
                if number == "0":
                    previous_timestamps[name] = (message, time)
                    continue
                prev_message, prev = previous_timestamps[name]
                previous_timestamps[name] = (message, time)
                durations[prev_message].append(time - prev)
                cur_durations[prev_message] = time - prev

    for message in durations:
        print("{:8} {}".format((sum(durations[message], start=timedelta()) / len(durations[message])) / timedelta(milliseconds=1), message))

    print()

    """
    for message in durations:
        print("{:8} {}".format(max(durations[message]) / timedelta(milliseconds=1), message))

    print()

    for message in bad_durations[""]:
        print("{:8} {}".format((sum(durations[message], start=timedelta()) / len(durations[message])) / timedelta(milliseconds=1), message))

    print()
    """

    for message, dur in bad_durations["macro"].items():
        print("{:8} {}".format(max(dur) / timedelta(milliseconds=1), message))
    print()
    for message, dur in bad_durations["micro"].items():
        print("{:8} {}".format(max(dur) / timedelta(milliseconds=1), message))

if __name__ == "__main__":
    sys.exit(main())
