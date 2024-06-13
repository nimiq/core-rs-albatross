import argparse
from sh import cargo
from pathlib import Path

def publish_order():
    order = []
    try:
        # Use cargo tree to get dependency tree
        for line in cargo.tree(all=True, _cwd="client/"):
            parts = line.strip().split()
            if parts[-1] == "(*)":
                parts = parts[:-1]
            # Ensure parts have necessary fields
            if len(parts) < 3:
                continue
            crate, version, *path_parts = parts
            path = " ".join(path_parts)[1:-1]
            if path.startswith("http"):
                continue
            # Convert path to relative path
            path = Path(path).resolve().relative_to(Path.cwd())
            order.append((crate, version, path))
        # Reverse order to process from root to leaves
        order.reverse()
        seen = set()
        new_order = []
        for crate, version, path in order:
            if crate not in seen:
                seen.add(crate)
                new_order.append((crate, version, path))
        return new_order
    except Exception as e:
        print(f"Error in publish_order: {e}")
        return []

def print_files(crates):
    try:
        for crate, version, path in crates:
            print(f"# Packaging {crate} {version}: {path}")
            # Use cargo package command to list package details
            for l in cargo.package(list=True, _cwd=str(path)):
                print(l.strip())
            print()
    except Exception as e:
        print(f"Error in print_files: {e}")

def print_paths(crates):
    try:
        for _, _, path in crates:
            print(path)
    except Exception as e:
        print(f"Error in print_paths: {e}")

def main():
    parser = argparse.ArgumentParser(description="Process Rust project dependencies using cargo.")
    parser.add_argument('--list-files', action='store_true', help="List package details for each crate.")
    parser.add_argument('--list-paths', action='store_true', help="List paths of crates.")
    args = parser.parse_args()

    try:
        crates = publish_order()
        if args.list_files:
            print_files(crates)
        elif args.list_paths:
            print_paths(crates)
        else:
            print("No action specified. Use --list-files or --list-paths.")
    except Exception as e:
        print(f"Error in main: {e}")

if __name__ == "__main__":
    main()
