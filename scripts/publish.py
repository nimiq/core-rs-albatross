from sh import cargo, grep
from pathlib import Path

def publish_order():
    order = []
    for line in cargo.tree(no_indent=True, all=True, _cwd="client/"):
        parts = line.strip().split()
        if parts[-1] == "(*)":
            parts = parts[:-1]
        if len(parts) == 2:
            continue
        crate, version, path = parts
        path = path[1:-1]
        if path.startswith("http"):
            continue
        path = Path(path).relative_to(Path.cwd())
        order.append((crate, version, path))
    order.reverse()
    seen = set()
    new_order = []
    for crate, version, path in order:
        if crate not in seen:
            seen.add(crate)
            new_order.append((crate, version, path))
    return new_order


def print_files(crates):
    for crate, version, path in crates:
        print("# Packaging {} {}: {}".format(crate, version, path))
        for l in cargo.package(list=True, _cwd=str(path)):
            print(l.strip())
        print()

def print_paths(crates):
    for _, _, path in crates:
        print(path)


crates = publish_order()
#print_files(crates)
print_paths(crates)